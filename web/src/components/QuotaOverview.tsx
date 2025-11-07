import type { CSSProperties } from 'react'
import type { QuotaSnapshot } from '../lib/api'
import { AnimatedDigits } from './AnimatedDigits'
import { useTranslation } from '../i18n'

type RadialProgressStyle = CSSProperties & {
  '--value': number
  '--size': string
  '--thickness': string
}

interface QuotaOverviewProps {
  snapshot: QuotaSnapshot | null
  isLoading: boolean
  error?: string | null
}

function formatCurrency(value?: number) {
  if (value === undefined || Number.isNaN(value)) return '—'
  return `$${value.toFixed(2)}`
}

function formatDate(value?: string) {
  if (!value) return '—'
  const parsed = new Date(value)
  if (!Number.isNaN(parsed.getTime())) {
    const pad = (n: number) => String(n).padStart(2, '0')
    return `${parsed.getFullYear()}-${pad(parsed.getMonth() + 1)}-${pad(parsed.getDate())} ${pad(parsed.getHours())}:${pad(parsed.getMinutes())}:${pad(parsed.getSeconds())}`
  }
  const match = value.match(/^([0-9]{4}-[0-9]{2}-[0-9]{2})[ T]([0-9]{2}:[0-9]{2}:[0-9]{2})/)
  if (match) {
    return `${match[1]} ${match[2]}`
  }
  return value.replace('T', ' ').replace(/\..*/, '').replace(/Z|[+-][0-9]{2}:[0-9]{2}$/g, '')
}

function calcUsagePercent(limit?: number, used?: number) {
  if (!limit || limit === 0 || used === undefined) return 0
  return Math.min(100, Math.max(0, (used / limit) * 100))
}

export function QuotaOverview({ snapshot, isLoading, error }: QuotaOverviewProps) {
  const { t } = useTranslation()

  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  const amountLimit = snapshot?.amountLimit ?? snapshot?.usedAmount ?? 0
  const usedAmount = snapshot?.usedAmount ?? 0
  const remainingAmount = snapshot?.remainingAmount ?? amountLimit - usedAmount
  const usagePercent = calcUsagePercent(amountLimit, usedAmount)

  const radialProgressStyle: RadialProgressStyle = {
    '--value': usagePercent,
    // Smaller size per feedback
    '--size': '5.4rem',
    '--thickness': '0.45rem',
  }

  return (
    <div className="card h-full bg-base-100 shadow-sm">
      <div className="card-body gap-6">
        {/* 顶部仅保留状态徽章；去掉标题与说明 */}
        <div className="flex items-center justify-end min-h-6">
          <span className="badge badge-success badge-sm" hidden={!snapshot?.isActive}>
            {t('quota.status.active')}
          </span>
        </div>

        <div className="grid gap-3 grid-cols-2 items-stretch">
          <OverviewTile label={t('quota.labels.usageRate')} compact padClass="px-4 py-2" overlayLabel>
            {/* Center the radial progress; minimize vertical gap to ~py-2 via tile padding */}
            <div className="flex items-center justify-center h-full">
              <div className="radial-progress text-primary" style={radialProgressStyle}>
                {isLoading ? '…' : <AnimatedDigits value={`${Math.round(usagePercent)}%`} />}
              </div>
            </div>
          </OverviewTile>
          <OverviewTile label={t('quota.labels.used')} loading={isLoading}>
            {isLoading ? '…' : <AnimatedDigits value={formatCurrency(usedAmount)} />}
          </OverviewTile>
          <OverviewTile label={t('quota.labels.remaining')} loading={isLoading}>
            {isLoading ? '…' : <AnimatedDigits value={formatCurrency(remainingAmount)} />}
          </OverviewTile>
          <OverviewTile
            label={t('quota.labels.nextReset')}
            value={formatDate(snapshot?.periodResetTime)}
            loading={isLoading}
            compact
          />
        </div>
      </div>
    </div>
  )
}

interface OverviewTileProps {
  label: string
  value?: string
  caption?: string
  loading?: boolean
  compact?: boolean
  children?: React.ReactNode
  padClass?: string
  overlayLabel?: boolean
}

function OverviewTile({ label, value, caption, loading, compact, children, padClass, overlayLabel }: OverviewTileProps) {
  const valueClass = compact
    ? 'text-xl font-semibold whitespace-nowrap overflow-hidden text-ellipsis'
    : 'text-2xl font-semibold whitespace-nowrap overflow-hidden text-ellipsis'
  return (
    <div className={`rounded-box border border-base-300 bg-base-200/60 ${overlayLabel ? 'relative' : ''} ${padClass ?? 'p-4'}`}>
      {overlayLabel ? (
        <div className="absolute top-2 left-2 text-sm text-base-content/60">{label}</div>
      ) : (
        <div className="text-sm text-base-content/60">{label}</div>
      )}
      <div className={valueClass}>{loading ? <span className="loading loading-dots loading-md" /> : (children ?? value)}</div>
      {caption ? <div className="text-xs text-base-content/50">{caption}</div> : null}
    </div>
  )
}

// CountdownUntil removed
