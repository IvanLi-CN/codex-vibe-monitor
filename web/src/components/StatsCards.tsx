import type { StatsResponse } from '../lib/api'
import { AdaptiveMetricValue } from './AdaptiveMetricValue'
import { useTranslation } from '../i18n'
import { Alert } from './ui/alert'

interface StatsCardsProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
}

export function StatsCards({ stats, loading, error }: StatsCardsProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'

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
        <div className="metric-value min-w-0 overflow-hidden text-primary">
          {loading ? '…' : <AdaptiveMetricValue value={totalCalls} localeTag={localeTag} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.success')}</div>
        <div className="metric-value min-w-0 overflow-hidden text-success">
          {loading ? '…' : <AdaptiveMetricValue value={successCount} localeTag={localeTag} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.failures')}</div>
        <div className="metric-value min-w-0 overflow-hidden text-error">
          {loading ? '…' : <AdaptiveMetricValue value={failureCount} localeTag={localeTag} />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.totalCost')}</div>
        <div className="metric-value min-w-0 overflow-hidden">
          {loading ? '…' : <AdaptiveMetricValue value={totalCost} localeTag={localeTag} kind="currency" />}
        </div>
      </div>
      <div className="metric-cell">
        <div className="metric-label">{t('stats.cards.totalTokens')}</div>
        <div className="metric-value min-w-0 overflow-hidden">
          {loading ? '…' : <AdaptiveMetricValue value={totalTokens} localeTag={localeTag} />}
        </div>
      </div>
    </div>
  )
}
