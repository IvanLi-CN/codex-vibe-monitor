import type { CSSProperties } from 'react'
import type { QuotaSnapshot, StatsResponse } from '../lib/api'

type RadialProgressStyle = CSSProperties & {
  '--value': number
  '--size': string
  '--thickness': string
}

interface QuotaOverviewProps {
  snapshot: QuotaSnapshot | null
  isLoading: boolean
  summary24h: StatsResponse | null
  summaryLoading: boolean
  error?: string | null
  summaryError?: string | null
}

function formatCurrency(value?: number) {
  if (value === undefined || Number.isNaN(value)) return '—'
  return `$${value.toFixed(2)}`
}

function formatNumber(value?: number) {
  if (value === undefined || Number.isNaN(value)) return '—'
  if (Math.abs(value) >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(2)}M`
  }
  if (Math.abs(value) >= 1_000) {
    return `${(value / 1_000).toFixed(2)}K`
  }
  return value.toLocaleString()
}

function formatDate(value?: string) {
  if (!value) return '—'
  return value.replace('T', ' ').replace('Z', '')
}

function calcUsagePercent(limit?: number, used?: number) {
  if (!limit || limit === 0 || used === undefined) return 0
  return Math.min(100, Math.max(0, (used / limit) * 100))
}

function formatPercent(value?: number) {
  if (value === undefined || Number.isNaN(value)) return '—'
  return `${(value * 100).toFixed(1)}%`
}

export function QuotaOverview({
  snapshot,
  isLoading,
  summary24h,
  summaryLoading,
  error,
  summaryError,
}: QuotaOverviewProps) {
  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  const amountLimit = snapshot?.amountLimit ?? snapshot?.usedAmount ?? 0
  const usedAmount = snapshot?.usedAmount ?? 0
  const remainingAmount = snapshot?.remainingAmount ?? (amountLimit - usedAmount)
  const usagePercent = calcUsagePercent(amountLimit, usedAmount)
  let successRate: number | undefined
  if (summary24h && summary24h.totalCount > 0) {
    successRate = summary24h.successCount / summary24h.totalCount
  }

  const radialProgressStyle: RadialProgressStyle = {
    '--value': usagePercent,
    '--size': '6rem',
    '--thickness': '0.6rem',
  }

  return (
    <div className="card bg-base-100 shadow-sm">
      <div className="card-body gap-6">
        <div className="flex flex-wrap items-center justify-between">
          <div>
            <h2 className="card-title">配额概览</h2>
            <p className="text-sm text-base-content/60">订阅：{snapshot?.subTypeName ?? '—'}</p>
          </div>
          <div className="flex items-center gap-4">
            <div className="text-sm text-base-content/60">
              <span className="badge badge-success badge-sm" hidden={!snapshot?.isActive}>
                正常使用
              </span>
            </div>
          </div>
        </div>

        <div className="grid gap-6 lg:grid-cols-3">
          <section className="space-y-4">
            <SectionTitle>套餐信息</SectionTitle>
            <div className="flex items-center justify-center">
              <div
                className="radial-progress text-primary"
                style={radialProgressStyle}
              >
                {isLoading ? '…' : `${Math.round(usagePercent)}%`}
              </div>
            </div>
            <div className="grid gap-3 sm:grid-cols-2">
              <OverviewTile label="总额度" value={formatCurrency(amountLimit)} caption="按天结算" loading={isLoading} />
              <OverviewTile label="计费类型" value={snapshot?.billingType ?? '—'} loading={isLoading} />
              <OverviewTile label="订阅 ID" value={snapshot?.subTypeName ?? '—'} loading={isLoading} />
            </div>
          </section>

          <section className="space-y-4">
            <SectionTitle>当前重置周期</SectionTitle>
            <div className="grid gap-3 sm:grid-cols-2">
              <OverviewTile label="已使用" value={formatCurrency(usedAmount)} loading={isLoading} />
              <OverviewTile label="剩余额度" value={formatCurrency(remainingAmount)} loading={isLoading} />
              <OverviewTile label="下次重置" value={formatDate(snapshot?.periodResetTime)} loading={isLoading} />
              <OverviewTile label="到期时间" value={formatDate(snapshot?.expireTime)} loading={isLoading} />
              <OverviewTile label="允许使用" value={snapshot?.period ?? '—'} loading={isLoading} />
              <OverviewTile label="可用次数" value={formatNumber(snapshot?.remainingCount)} loading={isLoading} />
            </div>
          </section>

          <section className="space-y-4">
            <SectionTitle>最近 24 小时</SectionTitle>
            {summaryError ? (
              <div className="alert alert-warning text-sm">
                <span>统计数据暂不可用：{summaryError}</span>
              </div>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2">
                <OverviewTile
                  label="总调用"
                  value={formatNumber(summary24h?.totalCount)}
                  loading={summaryLoading}
                />
                <OverviewTile
                  label="成功次数"
                  value={formatNumber(summary24h?.successCount)}
                  loading={summaryLoading}
                />
                <OverviewTile
                  label="失败次数"
                  value={formatNumber(summary24h?.failureCount)}
                  loading={summaryLoading}
                />
                <OverviewTile
                  label="成功率"
                  value={formatPercent(successRate)}
                  loading={summaryLoading}
                />
                <OverviewTile
                  label="费用"
                  value={formatCurrency(summary24h?.totalCost)}
                  loading={summaryLoading}
                />
                <OverviewTile
                  label="Token"
                  value={formatNumber(summary24h?.totalTokens)}
                  loading={summaryLoading}
                />
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  )
}

interface OverviewTileProps {
  label: string
  value: string
  caption?: string
  loading?: boolean
}

function OverviewTile({ label, value, caption, loading }: OverviewTileProps) {
  return (
    <div className="rounded-box border border-base-300 bg-base-200/60 p-4">
      <div className="text-sm text-base-content/60">{label}</div>
      <div className="text-2xl font-semibold">
        {loading ? <span className="loading loading-dots loading-md" /> : value}
      </div>
      {caption ? <div className="text-xs text-base-content/50">{caption}</div> : null}
    </div>
  )
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <h3 className="text-sm font-semibold uppercase text-base-content/60">{children}</h3>
}
