import type { KeyboardEvent, ReactNode } from 'react'
import { AppIcon } from './AppIcon'
import { MotherAccountBadge } from './MotherAccountToggle'
import { Badge } from './ui/badge'
import type { AccountTagSummary, UpstreamAccountSummary } from '../lib/api'
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
    group: string
    windows: string
    primary: string
    primaryShort: string
    secondary: string
    secondaryShort: string
    nextReset: string
    oauth: string
    apiKey: string
    mother: string
    duplicate: string
    off: string
    status: (value: string) => string
  }
}

function windowPercent(value?: number | null) {
  if (!Number.isFinite(value ?? NaN)) return 0
  return Math.max(0, Math.min(value ?? 0, 100))
}

function formatDateTime(value?: string | null, fallback = '—') {
  if (!value) return fallback
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }).format(date)
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

function compactBadge(content: ReactNode, variant: 'accent' | 'secondary' | 'success' | 'warning' | 'error' | 'info') {
  return (
    <Badge variant={variant} className="shrink-0 whitespace-nowrap px-2 py-0 text-[11px] font-medium">
      {content}
    </Badge>
  )
}

function renderTagBadges(tags?: AccountTagSummary[] | null) {
  const safeTags = tags ?? []
  const visible = safeTags.slice(0, 3)
  const overflowCount = safeTags.length - visible.length

  return (
    <>
      {visible.map((tag) => (
        <Badge
          key={tag.id}
          variant="secondary"
          className="shrink-0 whitespace-nowrap px-2 py-0 text-[11px] font-medium"
          title={tag.name}
        >
          {tag.name}
        </Badge>
      ))}
      {overflowCount > 0
        ? compactBadge(`+${overflowCount}`, 'secondary')
        : null}
    </>
  )
}

function CompactWindowLine({
  label,
  percent,
  text,
  resetText,
  accentClassName,
}: {
  label: string
  percent: number
  text: string
  resetText?: string
  accentClassName?: string
}) {
  const summary = resetText ? `${text} · ${resetText}` : text

  return (
    <div className="grid grid-cols-[2.1rem,minmax(0,1fr),5.5rem,2.75rem] items-center gap-1.5">
      <span className="truncate whitespace-nowrap text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/48">
        {label}
      </span>
      <span className="truncate whitespace-nowrap text-[11px] text-base-content/68" title={summary}>
        {summary}
      </span>
      <div className="h-1.5 overflow-hidden rounded-full bg-base-300/60">
        <div
          className={cn('h-full rounded-full bg-primary', accentClassName)}
          style={{ width: `${percent}%` }}
        />
      </div>
      <span className="truncate text-right text-xs font-semibold text-base-content/78">
        {Math.round(percent)}%
      </span>
    </div>
  )
}

function handleRowKeyDown(
  event: KeyboardEvent<HTMLTableRowElement>,
  accountId: number,
  onSelect: (accountId: number) => void,
) {
  if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    onSelect(accountId)
  }
}

export function UpstreamAccountsTable({
  items,
  selectedId,
  onSelect,
  emptyTitle,
  emptyDescription,
  labels,
}: UpstreamAccountsTableProps) {
  if (items.length === 0) {
    return (
      <div className="flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 py-10 text-center">
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <AppIcon name="server-network-outline" className="h-7 w-7" aria-hidden />
        </div>
        <h3 className="text-lg font-semibold text-base-content">
          {emptyTitle}
        </h3>
        <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
          {emptyDescription}
        </p>
      </div>
    )
  }

  return (
    <div className="overflow-hidden rounded-[1.35rem] border border-base-300/80 bg-base-100/72">
      <table className="w-full table-fixed border-collapse">
        <colgroup>
          <col className="w-[41%]" />
          <col className="w-[13%]" />
          <col className="w-[42%]" />
          <col className="w-[4%]" />
        </colgroup>
        <thead>
          <tr className="border-b border-base-300/80 bg-base-100/86 text-left">
            <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              Account
            </th>
            <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              {labels.sync}
            </th>
            <th className="px-4 py-3 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              {labels.windows}
            </th>
            <th className="w-12 px-4 py-3" aria-hidden />
          </tr>
        </thead>
        <tbody>
          {items.map((item, index) => {
            const primary = windowPercent(item.primaryWindow?.usedPercent)
            const secondary = windowPercent(item.secondaryWindow?.usedPercent)
            const primaryResetText = item.primaryWindow?.resetsAt
              ? `${labels.nextReset} ${formatDateTime(item.primaryWindow.resetsAt)}`
              : undefined
            const secondaryResetText = item.secondaryWindow?.resetsAt
              ? `${labels.nextReset} ${formatDateTime(item.secondaryWindow.resetsAt)}`
              : undefined
            const selected = item.id === selectedId
            const groupValue = item.groupName?.trim() || '—'
            return (
              <tr
                key={item.id}
                role="button"
                tabIndex={0}
                aria-pressed={selected}
                onClick={() => onSelect(item.id)}
                onKeyDown={(event) => handleRowKeyDown(event, item.id, onSelect)}
                className={cn(
                  'cursor-pointer border-b border-base-300/70 align-top outline-none transition-colors last:border-b-0 hover:bg-base-100/88 focus-visible:bg-base-100/88',
                  selected && 'bg-primary/10',
                  index % 2 === 1 && !selected && 'bg-base-100/32',
                )}
              >
                <td className="px-4 py-4">
                  <div className="min-w-0">
                    <p
                      className="truncate whitespace-nowrap text-base font-semibold text-base-content"
                      title={item.displayName}
                    >
                      {item.displayName}
                    </p>
                    <p
                      className="mt-1 truncate whitespace-nowrap text-sm text-base-content/62"
                      title={`${labels.group}: ${groupValue}`}
                    >
                      {groupValue}
                    </p>
                    <div className="mt-2 flex min-w-0 items-center gap-1.5 overflow-hidden">
                      {item.isMother ? (
                        <div className="shrink-0">
                          <MotherAccountBadge label={labels.mother} />
                        </div>
                      ) : null}
                      {item.duplicateInfo
                        ? compactBadge(labels.duplicate, 'warning')
                        : null}
                      {compactBadge(labels.status(item.status), badgeVariant(item.status))}
                      {!item.enabled
                        ? compactBadge(labels.off, 'secondary')
                        : null}
                      {compactBadge(kindLabel(item, labels), 'secondary')}
                      {item.planType
                        ? compactBadge(item.planType, 'accent')
                        : null}
                      {renderTagBadges(item.tags)}
                    </div>
                  </div>
                </td>
                <td className="px-4 py-4 align-middle">
                  <span
                    className="block truncate whitespace-nowrap text-sm text-base-content/72"
                    title={formatDateTime(item.lastSuccessfulSyncAt, labels.never)}
                  >
                    {formatDateTime(item.lastSuccessfulSyncAt, labels.never)}
                  </span>
                </td>
                <td className="px-4 py-4">
                  <div className="space-y-2">
                    <CompactWindowLine
                      label={labels.primaryShort}
                      percent={primary}
                      text={item.primaryWindow?.usedText ?? '—'}
                      resetText={primaryResetText}
                    />
                    <CompactWindowLine
                      label={labels.secondaryShort}
                      percent={secondary}
                      text={item.secondaryWindow?.usedText ?? '—'}
                      resetText={secondaryResetText}
                      accentClassName="bg-secondary"
                    />
                  </div>
                </td>
                <td className="px-4 py-4 text-right align-middle">
                  <AppIcon
                    name={selected ? 'chevron-right-circle' : 'chevron-right'}
                    className={cn('h-5 w-5', selected ? 'text-primary' : 'text-base-content/35')}
                    aria-hidden
                  />
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
