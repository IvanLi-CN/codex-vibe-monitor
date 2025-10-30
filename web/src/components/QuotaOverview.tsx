import type { QuotaSnapshot } from '../lib/api'

interface QuotaOverviewProps {
  snapshot: QuotaSnapshot | null
  isLoading: boolean
  error?: string | null
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

export function QuotaOverview({ snapshot, isLoading, error }: QuotaOverviewProps) {
  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  const amountLimit = snapshot?.amountLimit ?? snapshot?.usedAmount ?? 0
  const usedAmount = snapshot?.usedAmount ?? 0
  const remainingAmount = snapshot?.remainingAmount ?? (amountLimit - usedAmount)
  const usagePercent = calcUsagePercent(amountLimit, usedAmount)

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

        <div className="grid gap-6 md:grid-cols-5">
          <div className="flex items-center justify-center">
            <div className="radial-progress text-primary" style={{ ['--value' as any]: usagePercent, ['--size' as any]: '6rem', ['--thickness' as any]: '0.6rem' }}>
              {isLoading ? '…' : `${Math.round(usagePercent)}%`}
            </div>
          </div>

          <div className="md:col-span-4 grid gap-4 md:grid-cols-2">
            <OverviewTile label="总额度" value={formatCurrency(amountLimit)} caption="按天结算" loading={isLoading} />
            <OverviewTile label="已使用" value={formatCurrency(usedAmount)} loading={isLoading} />
            <OverviewTile label="剩余额度" value={formatCurrency(remainingAmount)} loading={isLoading} />
            <OverviewTile
              label="到期时间"
              value={formatDate(snapshot?.expireTime)}
              loading={isLoading}
            />
            <OverviewTile
              label="下次重置"
              value={formatDate(snapshot?.periodResetTime)}
              loading={isLoading}
            />
            <OverviewTile
              label="计费类型"
              value={snapshot?.billingType ?? '—'}
              loading={isLoading}
            />
          </div>
        </div>

        <div className="grid gap-4 md:grid-cols-4">
          <OverviewTile
            label="总调用次数"
            value={formatNumber(snapshot?.totalRequests)}
            loading={isLoading}
          />
          <OverviewTile
            label="Token 消耗"
            value={formatNumber(snapshot?.totalTokens)}
            loading={isLoading}
          />
          <OverviewTile
            label="费用消耗"
            value={formatCurrency(snapshot?.totalCost)}
            loading={isLoading}
          />
          <OverviewTile
            label="最后请求时间"
            value={formatDate(snapshot?.lastRequestTime)}
            loading={isLoading}
          />
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
