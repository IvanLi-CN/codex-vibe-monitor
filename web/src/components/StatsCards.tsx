import { useMemo } from 'react'
import type { StatsResponse } from '../lib/api'
import { AnimatedDigits } from './AnimatedDigits'
import { useTranslation } from '../i18n'
import { Alert } from './ui/alert'

interface StatsCardsProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
}

export function StatsCards({ stats, loading, error }: StatsCardsProps) {
  const { t, locale } = useTranslation()
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(locale === 'zh' ? 'zh-CN' : 'en-US', { maximumFractionDigits: 2 }),
    [locale],
  )

  if (error) {
    return (
      <Alert variant="error">
        <span>{t('stats.cards.loadError', { error })}</span>
      </Alert>
    )
  }

  const totalCalls = stats?.totalCount ?? 0
  const successCount = stats?.successCount ?? 0
  const failureCount = stats?.failureCount ?? 0
  const totalCost = stats?.totalCost ?? 0
  const totalTokens = stats?.totalTokens ?? 0

  return (
    <div className="metric-grid">
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.totalCalls')}</div>
        <div className="metric-value text-primary">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(totalCalls)} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.success')}</div>
        <div className="metric-value text-success">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(successCount)} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.failures')}</div>
        <div className="metric-value text-error">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(failureCount)} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.totalCost')}</div>
        <div className="metric-value">
          {loading ? '…' : (
            <span>
              $
              <AnimatedDigits value={numberFormatter.format(totalCost)} />
            </span>
          )}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.totalTokens')}</div>
        <div className="metric-value">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(totalTokens)} />}
        </div>
      </div>
    </div>
  )
}
