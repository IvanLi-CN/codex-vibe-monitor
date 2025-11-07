import { useMemo } from 'react'
import type { StatsResponse } from '../lib/api'
import { AnimatedDigits } from './AnimatedDigits'
import { useTranslation } from '../i18n'

interface StatsCardsProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
}

export function StatsCards({ stats, loading }: StatsCardsProps) {
  const { t, locale } = useTranslation()
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(locale === 'zh' ? 'zh-CN' : 'en-US', { maximumFractionDigits: 2 }),
    [locale],
  )

  const totalCalls = stats?.totalCount ?? 0
  const successCount = stats?.successCount ?? 0
  const failureCount = stats?.failureCount ?? 0
  const totalCost = stats?.totalCost ?? 0
  const totalTokens = stats?.totalTokens ?? 0

  return (
    <div className="stats shadow bg-base-100">
      <div className="stat">
        <div className="stat-title">{t('stats.cards.totalCalls')}</div>
        <div className="stat-value text-primary">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(totalCalls)} />}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">{t('stats.cards.success')}</div>
        <div className="stat-value text-success">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(successCount)} />}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">{t('stats.cards.failures')}</div>
        <div className="stat-value text-error">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(failureCount)} />}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">{t('stats.cards.totalCost')}</div>
        <div className="stat-value">
          {loading ? '…' : (
            <span>
              $
              <AnimatedDigits value={numberFormatter.format(totalCost)} />
            </span>
          )}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">{t('stats.cards.totalTokens')}</div>
        <div className="stat-value">
          {loading ? '…' : <AnimatedDigits value={numberFormatter.format(totalTokens)} />}
        </div>
      </div>
    </div>
  )
}
