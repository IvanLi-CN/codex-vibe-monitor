import { useEffect, useMemo, useState } from 'react'
import type { CSSProperties } from 'react'
import type { QuotaSnapshot } from '../lib/api'
import { AnimatedDigits } from './AnimatedDigits'

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

// number formatting not needed after removing count fields

function formatDate(value?: string) {
  if (!value) return '—'
  // Prefer robust parsing and output as YYYY-MM-DD HH:mm:ss (single line)
  const parsed = new Date(value)
  if (!Number.isNaN(parsed.getTime())) {
    const pad = (n: number) => String(n).padStart(2, '0')
    const y = parsed.getFullYear()
    const m = pad(parsed.getMonth() + 1)
    const d = pad(parsed.getDate())
    const hh = pad(parsed.getHours())
    const mm = pad(parsed.getMinutes())
    const ss = pad(parsed.getSeconds())
    return `${y}-${m}-${d} ${hh}:${mm}:${ss}`
  }
  // Fallback: strip milliseconds and timezone, keep up to seconds
  const m = value.match(/^(\d{4}-\d{2}-\d{2})[ T](\d{2}:\d{2}:\d{2})/)
  if (m) return `${m[1]} ${m[2]}`
  return value.replace('T', ' ').replace(/\.\d+/, '').replace(/Z|[+-]\d{2}:\d{2}$/g, '')
}

function calcUsagePercent(limit?: number, used?: number) {
  if (!limit || limit === 0 || used === undefined) return 0
  return Math.min(100, Math.max(0, (used / limit) * 100))
}

export function QuotaOverview({
  snapshot,
  isLoading,
  error,
}: QuotaOverviewProps) {
  if (error) {
    return <div className="alert alert-error">{error}</div>
  }

  const amountLimit = snapshot?.amountLimit ?? snapshot?.usedAmount ?? 0
  const usedAmount = snapshot?.usedAmount ?? 0
  const remainingAmount = snapshot?.remainingAmount ?? (amountLimit - usedAmount)
  const usagePercent = calcUsagePercent(amountLimit, usedAmount)

  const radialProgressStyle: RadialProgressStyle = {
    '--value': usagePercent,
    '--size': '4rem',
    '--thickness': '0.5rem',
  }

  return (
    <div className="card h-full bg-base-100 shadow-sm">
      <div className="card-body gap-6">
        <div className="flex flex-wrap items-center justify-between">
          <div>
            <h2 className="card-title">配额概览</h2>
            <p className="text-sm text-base-content/60 flex items-center gap-2">
              <span>订阅：{snapshot?.subTypeName ?? '—'}</span>
              <CountdownUntil expireISO={snapshot?.expireTime} />
            </p>
          </div>
          <div className="flex items-center gap-4">
            <div className="text-sm text-base-content/60">
              <span className="badge badge-success badge-sm" hidden={!snapshot?.isActive}>
                正常使用
              </span>
            </div>
          </div>
        </div>

        <div className="grid gap-3 grid-cols-2 items-stretch">
          <OverviewTile label="使用率" compact>
            <div className="flex items-center gap-3">
              <div className="radial-progress text-primary" style={radialProgressStyle}>
                {isLoading ? '…' : <AnimatedDigits value={`${Math.round(usagePercent)}%`} />}
              </div>
            </div>
          </OverviewTile>
          <OverviewTile label="已使用" loading={isLoading}>
            {isLoading ? '…' : <AnimatedDigits value={formatCurrency(usedAmount)} />}
          </OverviewTile>
          <OverviewTile label="剩余额度" loading={isLoading}>
            {isLoading ? '…' : <AnimatedDigits value={formatCurrency(remainingAmount)} />}
          </OverviewTile>
          <OverviewTile label="下次重置" value={formatDate(snapshot?.periodResetTime)} loading={isLoading} compact />
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
}

function OverviewTile({ label, value, caption, loading, compact, children }: OverviewTileProps) {
  const valueClass = compact
    ? 'text-xl font-semibold whitespace-nowrap overflow-hidden text-ellipsis'
    : 'text-2xl font-semibold whitespace-nowrap overflow-hidden text-ellipsis'
  return (
    <div className="rounded-box border border-base-300 bg-base-200/60 p-4">
      <div className="text-sm text-base-content/60">{label}</div>
      <div className={valueClass}>{loading ? <span className="loading loading-dots loading-md" /> : (children ?? value)}</div>
      {caption ? <div className="text-xs text-base-content/50">{caption}</div> : null}
    </div>
  )
}

function CountdownUntil({ expireISO }: { expireISO?: string }) {
  const [showAbsolute, setShowAbsolute] = useState(false)
  const [now, setNow] = useState(() => new Date())

  // tick interval depends on remaining time
  useEffect(() => {
    const tick = () => setNow(new Date())
    const id = setInterval(tick, 30_000)
    return () => clearInterval(id)
  }, [])

  const expire = useMemo(() => (expireISO ? new Date(expireISO) : null), [expireISO])
  const remaining = useMemo(() => (expire ? expire.getTime() - now.getTime() : NaN), [expire, now])

  const isExpired = Number.isFinite(remaining) && remaining <= 0
  const minutes = Number.isFinite(remaining) ? Math.ceil(remaining / 60_000) : NaN
  const hours = Number.isFinite(remaining) ? Math.ceil(remaining / 3_600_000) : NaN
  const days = Number.isFinite(remaining) ? Math.ceil(remaining / 86_400_000) : NaN

  let display = '—'
  let tone = 'text-base-content/60'
  if (expire) {
    if (isExpired) {
      display = '已到期'
      tone = 'text-error'
    } else if (Number.isFinite(days) && (days as number) >= 2) {
      display = `到期：剩余${days}天`
    } else if (Number.isFinite(hours) && (minutes as number) >= 100) {
      display = `到期：剩余${hours}小时`
      tone = 'text-warning'
    } else if (Number.isFinite(minutes)) {
      const mins = Math.max(1, minutes as number)
      display = `到期：剩余${mins}分钟`
      tone = 'text-warning'
    }
  }

  const absolute = expire ? `到期：${formatDate(expireISO!)}` : '到期：—'

  return (
    <span
      className={`inline-flex items-center gap-1 cursor-pointer select-none ${tone}`}
      title={absolute}
      onMouseEnter={() => setShowAbsolute(true)}
      onMouseLeave={() => setShowAbsolute(false)}
      onClick={() => setShowAbsolute(v => !v)}
    >
      {showAbsolute ? absolute : display}
    </span>
  )
}

// SectionTitle removed as headings are no longer needed per design
