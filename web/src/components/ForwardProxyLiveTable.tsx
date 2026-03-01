import { useMemo } from 'react'
import { useTranslation } from '../i18n'
import type { ForwardProxyHourlyBucket, ForwardProxyLiveNode, ForwardProxyLiveStatsResponse, ForwardProxyWindowStats } from '../lib/api'
import { cn } from '../lib/utils'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface ForwardProxyLiveTableProps {
  stats: ForwardProxyLiveStatsResponse | null
  isLoading: boolean
  error?: string | null
}

function formatSuccessRate(value?: number) {
  if (value == null || Number.isNaN(value)) return '—'
  return `${(value * 100).toFixed(1)}%`
}

function formatLatency(value?: number) {
  if (value == null || Number.isNaN(value)) return '—'
  return `${value.toFixed(0)} ms`
}

function sumLast24h(node: ForwardProxyLiveNode) {
  return node.last24h.reduce(
    (acc, bucket) => {
      acc.success += bucket.successCount
      acc.failure += bucket.failureCount
      return acc
    },
    { success: 0, failure: 0 },
  )
}

function bucketTooltipLabel(bucket: ForwardProxyHourlyBucket, localeTag: string, successLabel: string, failureLabel: string) {
  const start = new Date(bucket.bucketStart)
  const end = new Date(bucket.bucketEnd)
  const formatter = new Intl.DateTimeFormat(localeTag, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
  const startLabel = Number.isNaN(start.getTime()) ? bucket.bucketStart : formatter.format(start)
  const endLabel = Number.isNaN(end.getTime()) ? bucket.bucketEnd : formatter.format(end)
  return `${startLabel} - ${endLabel}\n${successLabel}: ${bucket.successCount}\n${failureLabel}: ${bucket.failureCount}`
}

function WindowCell({ value }: { value: ForwardProxyWindowStats }) {
  return (
    <div className="space-y-0.5 text-[11px] leading-tight">
      <div>{formatSuccessRate(value.successRate)}</div>
      <div className="text-base-content/65">{formatLatency(value.avgLatencyMs)}</div>
    </div>
  )
}

export function ForwardProxyLiveTable({ stats, isLoading, error }: ForwardProxyLiveTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'

  const rowData = useMemo(
    () =>
      (stats?.nodes ?? []).map((node) => ({
        node,
        windows: [node.stats.oneMinute, node.stats.fifteenMinutes, node.stats.oneHour, node.stats.oneDay, node.stats.sevenDays],
        total24h: sumLast24h(node),
        maxBucketTotal24h: Math.max(...node.last24h.map((bucket) => bucket.successCount + bucket.failureCount), 0),
      })),
    [stats?.nodes],
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

  if (rowData.length === 0) {
    return <Alert>{t('live.proxy.table.empty')}</Alert>
  }

  return (
    <div className="overflow-x-auto rounded-xl border border-base-300/75 bg-base-100/55">
      <table className="w-full min-w-[58rem] table-fixed text-xs">
        <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
          <tr>
            <th className="w-[22%] px-3 py-3 text-left font-semibold">{t('live.proxy.table.proxy')}</th>
            <th className="w-[9%] px-2 py-3 text-center font-semibold">{t('live.proxy.table.oneMinute')}</th>
            <th className="w-[9%] px-2 py-3 text-center font-semibold">{t('live.proxy.table.fifteenMinutes')}</th>
            <th className="w-[9%] px-2 py-3 text-center font-semibold">{t('live.proxy.table.oneHour')}</th>
            <th className="w-[9%] px-2 py-3 text-center font-semibold">{t('live.proxy.table.oneDay')}</th>
            <th className="w-[9%] px-2 py-3 text-center font-semibold">{t('live.proxy.table.sevenDays')}</th>
            <th className="w-[42%] px-3 py-3 text-left font-semibold">{t('live.proxy.table.trend24h')}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-base-300/65">
          {rowData.map(({ node, windows, total24h, maxBucketTotal24h }) => (
            <tr key={node.key} className={cn('transition-colors hover:bg-primary/6', node.penalized && 'bg-warning/8')}>
              <td className="max-w-0 px-3 py-3 align-middle">
                <div className="min-w-0">
                  <div className="truncate whitespace-nowrap text-sm font-medium" title={node.displayName}>
                    {node.displayName}
                  </div>
                  <div className="mt-1 text-[11px] text-base-content/65">
                    {t('live.proxy.table.successShort', { count: total24h.success })}
                    {' / '}
                    {t('live.proxy.table.failureShort', { count: total24h.failure })}
                  </div>
                </div>
              </td>
              {windows.map((window, index) => (
                <td key={`${node.key}-${index}`} className="px-2 py-3 text-center align-middle">
                  <WindowCell value={window} />
                </td>
              ))}
              <td className="px-3 py-3 align-middle">
                <div className="space-y-1">
                  <div className="flex h-11 items-end gap-[2px]">
                    {node.last24h.map((bucket, index) => {
                      const total = bucket.successCount + bucket.failureCount
                      const successHeight = maxBucketTotal24h > 0 ? (bucket.successCount / maxBucketTotal24h) * 100 : 0
                      const failureHeight = maxBucketTotal24h > 0 ? (bucket.failureCount / maxBucketTotal24h) * 100 : 0
                      const emptyHeight = Math.max(0, 100 - Math.round(successHeight + failureHeight))
                      return (
                        <div
                          key={`${node.key}-${index}`}
                          className="flex h-10 w-[6px] flex-col overflow-hidden rounded-[2px] bg-base-300/45"
                          title={bucketTooltipLabel(
                            bucket,
                            localeTag,
                            t('stats.cards.success'),
                            t('stats.cards.failures'),
                          )}
                        >
                          <div
                            className="bg-transparent"
                            style={{ height: `${emptyHeight}%` }}
                          />
                          <div
                            className={cn(total > 0 ? 'bg-error/85' : 'bg-transparent')}
                            style={{ height: `${Math.round(failureHeight)}%` }}
                          />
                          <div
                            className={cn(total > 0 ? 'bg-success/85' : 'bg-transparent')}
                            style={{ height: `${Math.round(successHeight)}%` }}
                          />
                        </div>
                      )
                    })}
                  </div>
                  <div className="flex items-center justify-between text-[10px] text-base-content/65">
                    <span>{t('live.proxy.table.successShort', { count: total24h.success })}</span>
                    <span>{t('live.proxy.table.failureShort', { count: total24h.failure })}</span>
                  </div>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
