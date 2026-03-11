import { Icon } from '@iconify/react'
import { Badge } from './ui/badge'
import type { UpstreamAccountSummary } from '../lib/api'
import { cn } from '../lib/utils'

interface UpstreamAccountsTableProps {
  items: UpstreamAccountSummary[]
  selectedId: number | null
  onSelect: (accountId: number) => void
  emptyTitle: string
  emptyDescription: string
  labels: {
    sync: string
    never: string
    primary: string
    secondary: string
    oauth: string
    apiKey: string
    status: (value: string) => string
  }
}

function windowPercent(value?: number | null) {
  if (!Number.isFinite(value ?? NaN)) return 0
  return Math.max(0, Math.min(value ?? 0, 100))
}

function kindLabel(item: UpstreamAccountSummary, labels: UpstreamAccountsTableProps['labels']) {
  return item.kind === 'oauth_codex' ? labels.oauth : labels.apiKey
}

function badgeVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'active') return 'success'
  if (status === 'syncing') return 'warning'
  if (status === 'error' || status === 'needs_reauth') return 'error'
  return 'secondary'
}

function MiniBar({ label, percent, text }: { label: string; percent: number; text: string }) {
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-3 text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
        <span>{label}</span>
        <span>{Math.round(percent)}%</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-base-300/70">
        <div className="h-full rounded-full bg-primary" style={{ width: `${percent}%` }} />
      </div>
      <p className="text-xs text-base-content/60">{text}</p>
    </div>
  )
}

export function UpstreamAccountsTable({ items, selectedId, onSelect, emptyTitle, emptyDescription, labels }: UpstreamAccountsTableProps) {
  if (items.length === 0) {
    return (
      <div className="flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 py-10 text-center">
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Icon icon="mdi:server-network-outline" className="h-7 w-7" aria-hidden />
        </div>
        <h3 className="text-lg font-semibold text-base-content">{emptyTitle}</h3>
        <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">{emptyDescription}</p>
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {items.map((item) => {
        const primary = windowPercent(item.primaryWindow?.usedPercent)
        const secondary = windowPercent(item.secondaryWindow?.usedPercent)
        const selected = item.id === selectedId
        return (
          <button
            key={item.id}
            type="button"
            onClick={() => onSelect(item.id)}
            className={cn(
              'w-full rounded-[1.35rem] border px-4 py-4 text-left shadow-sm transition-transform hover:-translate-y-0.5',
              selected
                ? 'border-primary/45 bg-primary/10 shadow-[0_18px_38px_rgba(37,99,235,0.16)]'
                : 'border-base-300/80 bg-base-100/72 hover:border-base-300 hover:bg-base-100/92',
            )}
          >
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0 space-y-2">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="truncate text-base font-semibold text-base-content">{item.displayName}</h3>
                  <Badge variant={badgeVariant(item.status)}>{labels.status(item.status)}</Badge>
                  <Badge variant="secondary">{kindLabel(item, labels)}</Badge>
                  {item.planType ? <Badge variant="secondary">{item.planType}</Badge> : null}
                </div>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-base-content/60">
                  {item.email ? <span>{item.email}</span> : null}
                  {item.maskedApiKey ? <span>{item.maskedApiKey}</span> : null}
                  <span>
                    {labels.sync}: {item.lastSuccessfulSyncAt ?? labels.never}
                  </span>
                </div>
              </div>
              <Icon
                icon={selected ? 'mdi:chevron-right-circle' : 'mdi:chevron-right'}
                className={cn('h-5 w-5 shrink-0', selected ? 'text-primary' : 'text-base-content/35')}
                aria-hidden
              />
            </div>
            <div className="mt-4 grid gap-3 md:grid-cols-2">
              <MiniBar label={labels.primary} percent={primary} text={item.primaryWindow?.usedText ?? '—'} />
              <MiniBar label={labels.secondary} percent={secondary} text={item.secondaryWindow?.usedText ?? '—'} />
            </div>
          </button>
        )
      })}
    </div>
  )
}
