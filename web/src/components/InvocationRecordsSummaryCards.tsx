import { useMemo } from 'react'
import type { InvocationFocus, InvocationRecordsSummaryResponse } from '../lib/api'
import { AnimatedDigits } from './AnimatedDigits'
import { Alert } from './ui/alert'
import { useTranslation } from '../i18n'

interface InvocationRecordsSummaryCardsProps {
  focus: InvocationFocus
  summary: InvocationRecordsSummaryResponse | null
  isLoading: boolean
  error?: string | null
}

interface SummaryMetric {
  label: string
  value: string
  toneClass?: string
}

function MetricCell({ label, value, toneClass, loading }: SummaryMetric & { loading: boolean }) {
  return (
    <div className="metric-cell">
      <div className="metric-label">{label}</div>
      <div className={`metric-value ${toneClass ?? ''}`}>{loading ? '…' : <AnimatedDigits value={value} />}</div>
    </div>
  )
}

export function InvocationRecordsSummaryCards({ focus, summary, isLoading, error }: InvocationRecordsSummaryCardsProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }), [localeTag])
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

  const hasSummary = summary !== null
  const showBlockingError = Boolean(error) && !hasSummary
  const showInlineError = Boolean(error) && hasSummary

  if (showBlockingError) {
    return <Alert variant="error">{t('records.summary.loadError', { error: error ?? '' })}</Alert>
  }

  const formatNumber = (value?: number | null) => numberFormatter.format(value ?? 0)
  const formatMs = (value?: number | null) => (value == null ? '—' : `${numberFormatter.format(value)} ms`)
  const formatCost = (value?: number | null) => currencyFormatter.format(value ?? 0)

  const metrics: SummaryMetric[] = (() => {
    switch (focus) {
      case 'network':
        return [
          { label: t('records.summary.network.avgTtfb'), value: formatMs(summary?.network.avgTtfbMs), toneClass: 'text-info' },
          { label: t('records.summary.network.p95Ttfb'), value: formatMs(summary?.network.p95TtfbMs) },
          { label: t('records.summary.network.avgTotal'), value: formatMs(summary?.network.avgTotalMs), toneClass: 'text-primary' },
          { label: t('records.summary.network.p95Total'), value: formatMs(summary?.network.p95TotalMs) },
        ]
      case 'exception':
        return [
          { label: t('records.summary.exception.failures'), value: formatNumber(summary?.exception.failureCount), toneClass: 'text-error' },
          { label: t('records.summary.exception.service'), value: formatNumber(summary?.exception.serviceFailureCount) },
          { label: t('records.summary.exception.client'), value: formatNumber(summary?.exception.clientFailureCount) },
          { label: t('records.summary.exception.abort'), value: formatNumber(summary?.exception.clientAbortCount) },
          { label: t('records.summary.exception.actionable'), value: formatNumber(summary?.exception.actionableFailureCount), toneClass: 'text-warning' },
        ]
      case 'token':
      default:
        return [
          { label: t('records.summary.token.requests'), value: formatNumber(summary?.token.requestCount), toneClass: 'text-primary' },
          { label: t('records.summary.token.totalTokens'), value: formatNumber(summary?.token.totalTokens) },
          { label: t('records.summary.token.avgTokens'), value: formatNumber(summary?.token.avgTokensPerRequest) },
          { label: t('records.summary.token.cacheInput'), value: formatNumber(summary?.token.cacheInputTokens), toneClass: 'text-info' },
          { label: t('records.summary.token.totalCost'), value: formatCost(summary?.token.totalCost) },
        ]
    }
  })()

  return (
    <div className="space-y-3">
      {showInlineError ? <Alert variant="error">{t('records.summary.loadError', { error: error ?? '' })}</Alert> : null}
      <div className="metric-grid">
        {metrics.map((metric) => (
          <MetricCell key={metric.label} {...metric} loading={isLoading && !hasSummary} />
        ))}
      </div>
    </div>
  )
}
