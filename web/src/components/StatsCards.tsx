import type { StatsResponse } from '../lib/api'

interface StatsCardsProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
}

const numberFormatter = new Intl.NumberFormat('en-US', {
  maximumFractionDigits: 2,
})

export function StatsCards({ stats, loading, error }: StatsCardsProps) {
  if (error) {
    return (
      <div className="alert alert-error">
        <span>Failed to load stats: {error}</span>
      </div>
    )
  }

  return (
    <div className="stats shadow bg-base-100">
      <div className="stat">
        <div className="stat-title">Total Calls</div>
        <div className="stat-value text-primary">
          {loading ? '…' : numberFormatter.format(stats?.totalCount ?? 0)}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">Success</div>
        <div className="stat-value text-success">
          {loading ? '…' : numberFormatter.format(stats?.successCount ?? 0)}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">Failures</div>
        <div className="stat-value text-error">
          {loading ? '…' : numberFormatter.format(stats?.failureCount ?? 0)}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">Total Cost</div>
        <div className="stat-value">
          {loading ? '…' : `$${numberFormatter.format(stats?.totalCost ?? 0)}`}
        </div>
      </div>
      <div className="stat">
        <div className="stat-title">Total Tokens</div>
        <div className="stat-value">
          {loading ? '…' : numberFormatter.format(stats?.totalTokens ?? 0)}
        </div>
      </div>
    </div>
  )
}
