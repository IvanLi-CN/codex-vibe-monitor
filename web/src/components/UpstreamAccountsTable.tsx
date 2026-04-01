import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { AppIcon } from './AppIcon'
import { MotherAccountBadge } from './MotherAccountToggle'
import { Button } from './ui/button'
import { Spinner } from './ui/spinner'
import { Badge } from './ui/badge'
import { Tooltip } from './ui/tooltip'
import type { AccountTagSummary, UpstreamAccountSummary } from '../lib/api'
import { formatTokensShort } from '../lib/numberFormatters'
import { upstreamPlanBadgeRecipe } from '../lib/upstreamAccountBadges'
import { cn } from '../lib/utils'

type ActionDetailLabelResolver =
  | ((item: UpstreamAccountSummary) => string | null)
  | ((value?: string | null) => string | null)

interface UpstreamAccountsTableProps {
  items: UpstreamAccountSummary[]
  isLoading?: boolean
  error?: string | null
  loadingTitle?: string
  loadingDescription?: string
  errorTitle?: string
  retryLabel?: string
  onRetry?: () => void
  selectedId: number | null
  selectedAccountIds: Set<number>
  onSelect: (accountId: number) => void
  onToggleSelected: (accountId: number, checked: boolean) => void
  onToggleSelectAllCurrentPage: (checked: boolean) => void
  emptyTitle: string
  emptyDescription: string
  labels: {
    selectPage: string
    selectRow: (name: string) => string
    account: string
    sync: string
    lastSuccess: string
    lastCall: string
    routingBlock: string
    latestAction: string
    never: string
    windows: string
    primary: string
    primaryShort: string
    secondary: string
    secondaryShort: string
    nextReset: string
    nextResetCompact?: string
    requestsMetric: string
    tokensMetric: string
    costMetric: string
    inputTokensMetric: string
    outputTokensMetric: string
    cacheInputTokensMetric: string
    unknown: string
    unavailable: string
    oauth: string
    apiKey: string
    mother: string
    duplicate: string
    hiddenTagsA11y: (count: number, names: string) => string
    workStatus: (status: string) => string
    workStatusCount: (count: number) => string
    enableStatus: (status: string) => string
    healthStatus: (status: string) => string
    syncState: (status: string) => string
    action: (action?: string | null) => string | null
    compactSupport?: (item: UpstreamAccountSummary) => string | null
    compactSupportHint?: (item: UpstreamAccountSummary) => string | null
    actionSource: ActionDetailLabelResolver
    actionReason: ActionDetailLabelResolver
    latestActionFieldAction: string
    latestActionFieldSource: string
    latestActionFieldReason: string
    latestActionFieldHttpStatus: string
    latestActionFieldOccurredAt: string
    latestActionFieldMessage: string
  }
}

const WINDOW_PLACEHOLDER = '-'

function SelectAllCheckbox({
  checked,
  indeterminate,
  ariaLabel,
  onChange,
}: {
  checked: boolean
  indeterminate: boolean
  ariaLabel: string
  onChange: (checked: boolean) => void
}) {
  const ref = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    if (!ref.current) return
    ref.current.indeterminate = indeterminate
  }, [indeterminate])

  return (
    <input
      ref={ref}
      type="checkbox"
      className="h-4 w-4 cursor-pointer rounded border-base-300/90 bg-base-100 accent-primary"
      aria-label={ariaLabel}
      checked={checked}
      onChange={(event) => onChange(event.target.checked)}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    />
  )
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

function numberLocale() {
  return typeof navigator !== 'undefined' && navigator.language ? navigator.language : 'en-US'
}

function formatInteger(value: number) {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(value)
}

function formatCost(value: number) {
  const abs = Math.abs(value)
  const maximumFractionDigits = abs >= 10 ? 2 : abs >= 1 ? 3 : 4
  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: abs >= 10 ? 2 : 0,
    maximumFractionDigits,
  }).format(value)
}

function kindLabel(item: UpstreamAccountSummary, labels: UpstreamAccountsTableProps['labels']) {
  return item.kind === 'oauth_codex' ? labels.oauth : labels.apiKey
}

function accountEnableStatus(item: UpstreamAccountSummary) {
  return item.enableStatus ?? (item.enabled === false || item.displayStatus === 'disabled' ? 'disabled' : 'enabled')
}

function accountHealthStatus(item: UpstreamAccountSummary) {
  if (item.healthStatus) return item.healthStatus
  const legacyStatus = item.displayStatus ?? item.status
  if (
    legacyStatus === 'needs_reauth' ||
    legacyStatus === 'upstream_unavailable' ||
    legacyStatus === 'upstream_rejected' ||
    legacyStatus === 'error_other'
  ) {
    return legacyStatus
  }
  if (legacyStatus === 'error') {
    return 'error_other'
  }
  return 'normal'
}

function accountSyncState(item: UpstreamAccountSummary) {
  if (item.syncState) return item.syncState
  return (item.displayStatus ?? item.status) === 'syncing' ? 'syncing' : 'idle'
}

function enableBadgeVariant(status: string): 'success' | 'secondary' {
  return status === 'enabled' ? 'success' : 'secondary'
}

function workBadgeVariant(status: string): 'info' | 'warning' | 'secondary' {
  if (status === 'working') return 'info'
  if (status === 'degraded') return 'warning'
  if (status === 'rate_limited') return 'warning'
  return 'secondary'
}

function resolveAvailabilityBadge(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps['labels'],
) {
  const enableStatus = accountEnableStatus(item)
  const healthStatus = accountHealthStatus(item)
  const syncState = accountSyncState(item)

  if (
    item.workStatus === 'degraded' &&
    enableStatus === 'enabled' &&
    healthStatus === 'normal' &&
    syncState === 'idle'
  ) {
    return {
      label: labels.workStatus('degraded'),
      variant: workBadgeVariant('degraded'),
    }
  }

  if (
    item.workStatus === 'rate_limited' &&
    enableStatus === 'enabled' &&
    healthStatus === 'normal' &&
    syncState === 'idle'
  ) {
    return {
      label: labels.workStatus('rate_limited'),
      variant: workBadgeVariant('rate_limited'),
    }
  }

  if (enableStatus !== 'enabled' || healthStatus !== 'normal' || syncState !== 'idle') {
    return null
  }

  if (item.workStatus === 'working') {
    const activeConversationCount = item.activeConversationCount ?? 0
    return {
      label:
        activeConversationCount > 0
          ? labels.workStatusCount(activeConversationCount)
          : labels.workStatus('working'),
      variant: workBadgeVariant('working'),
    }
  }

  if ((item.workStatus ?? 'idle') === 'idle') {
    return {
      label: labels.workStatus('idle'),
      variant: workBadgeVariant('idle'),
    }
  }

  return null
}

function healthBadgeVariant(status: string): 'warning' | 'error' | 'secondary' {
  if (status === 'upstream_unavailable') return 'warning'
  if (
    status === 'needs_reauth' ||
    status === 'upstream_rejected' ||
    status === 'error_other' ||
    status === 'error'
  ) {
    return 'error'
  }
  return 'secondary'
}

function syncBadgeVariant(status: string): 'warning' | 'secondary' {
  return status === 'syncing' ? 'warning' : 'secondary'
}

function compactBadge(
  content: ReactNode,
  variant: 'default' | 'accent' | 'secondary' | 'success' | 'warning' | 'error' | 'info',
  options?: {
    className?: string
    dataPlan?: string
    title?: string
  },
) {
  return (
    <Badge
      variant={variant}
      className={cn('shrink-0 whitespace-nowrap px-2 py-px text-[11px] font-medium leading-4', options?.className)}
      data-plan={options?.dataPlan}
      title={options?.title}
    >
      {content}
    </Badge>
  )
}

function splitVisibleAndHiddenTags(tags?: AccountTagSummary[] | null) {
  const safeTags = tags ?? []
  const visible = safeTags.slice(0, 3)
  const hidden = safeTags.slice(visible.length)
  return {
    visible,
    hidden,
  }
}

function renderTagBadges(tags?: AccountTagSummary[] | null) {
  const { visible } = splitVisibleAndHiddenTags(tags)
  return (
    <>
      {visible.map((tag) => (
        <Badge
          key={tag.id}
          variant="secondary"
          className="min-w-0 max-w-[7.5rem] truncate border-base-300/90 bg-base-200/90 px-2 py-px text-[11px] font-medium leading-4 text-base-content/92"
          title={tag.name}
        >
          {tag.name}
        </Badge>
      ))}
    </>
  )
}

function renderTagOverflowBadge(
  labels: UpstreamAccountsTableProps['labels'],
  tags?: AccountTagSummary[] | null,
) {
  const { hidden } = splitVisibleAndHiddenTags(tags)
  const overflowCount = hidden.length
  const hiddenNames = hidden.map((tag) => tag.name).join(', ')
  if (overflowCount === 0) return null

  return (
    <Tooltip
      content={
        <div className="max-w-56 text-xs leading-5 text-base-content/80">
          {hiddenNames}
        </div>
      }
      triggerProps={{
        tabIndex: 0,
        'aria-label': labels.hiddenTagsA11y(overflowCount, hiddenNames),
      }}
    >
      {compactBadge(`+${overflowCount}`, 'secondary', { title: hiddenNames })}
    </Tooltip>
  )
}

function CompactWindowLine({
  window,
  label,
  percent,
  resetText,
  metricLabels,
  missing,
  hideLabelWhenMissing,
  accentClassName,
  title,
  labelClassName,
}: {
  window?: UpstreamAccountSummary['primaryWindow']
  label: string
  percent: number
  resetText?: string
  metricLabels: {
    requests: string
    tokens: string
    cost: string
    inputTokens: string
    outputTokens: string
    cacheInputTokens: string
  }
  missing?: boolean
  hideLabelWhenMissing?: boolean
  accentClassName?: string
  title?: string
  labelClassName?: string
}) {
  const hideLabel = missing && hideLabelWhenMissing
  const displayLabel = hideLabel ? '' : label
  const displayResetText = missing ? WINDOW_PLACEHOLDER : (resetText ?? WINDOW_PLACEHOLDER)
  const usage = window?.actualUsage ?? null
  const displayRequests = missing || !usage ? WINDOW_PLACEHOLDER : formatInteger(usage.requestCount)
  const displayTokens = missing || !usage ? WINDOW_PLACEHOLDER : formatTokensShort(usage.totalTokens, numberLocale())
  const displayCost = missing || !usage ? WINDOW_PLACEHOLDER : formatCost(usage.totalCost)
  const tokenTooltip = missing || !usage ? null : (
    <div className="min-w-[12rem] space-y-1.5">
      <div className="flex items-center justify-between gap-4">
        <span className="text-[11px] font-medium text-base-content/72">{metricLabels.inputTokens}</span>
        <span className="text-[11px] font-semibold font-mono tabular-nums text-base-content">{formatInteger(usage.inputTokens)}</span>
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="text-[11px] font-medium text-base-content/72">{metricLabels.outputTokens}</span>
        <span className="text-[11px] font-semibold font-mono tabular-nums text-base-content">{formatInteger(usage.outputTokens)}</span>
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="text-[11px] font-medium text-base-content/72">{metricLabels.cacheInputTokens}</span>
        <span className="text-[11px] font-semibold font-mono tabular-nums text-base-content">{formatInteger(usage.cacheInputTokens)}</span>
      </div>
    </div>
  )
  const summary = missing
    ? WINDOW_PLACEHOLDER
    : `${displayRequests} · ${displayTokens} · ${displayCost}${resetText ? ` · ${resetText}` : ''}`
  const renderMetric = ({
    label,
    value,
    tooltip,
  }: {
    label: string
    value: string
    tooltip?: ReactNode | null
  }) => {
    const metric = (
      <div className="inline-flex min-w-0 items-baseline gap-1.5">
        <span className="shrink-0 text-[10px] font-semibold uppercase tracking-[0.06em] leading-4 text-base-content/45">
          {label}
        </span>
        <span
          className={
            missing
              ? 'truncate whitespace-nowrap text-[11px] leading-4 text-base-content/55 font-mono tabular-nums'
              : 'truncate whitespace-nowrap text-[11px] leading-4 text-base-content/78 font-mono tabular-nums'
          }
        >
          {value}
        </span>
      </div>
    )
    if (!tooltip || missing) return metric
    return (
      <Tooltip
        content={tooltip}
        triggerProps={{
          tabIndex: 0,
          'aria-label': `${label}: ${value}`,
        }}
      >
        {metric}
      </Tooltip>
    )
  }

  return (
    <div
      className="grid grid-cols-[max-content,minmax(0,1fr)] items-start gap-x-2 gap-y-1"
      title={missing ? undefined : (title ?? summary)}
    >
      <span
        className={cn(
          'row-span-2 min-w-[2ch] truncate whitespace-nowrap pt-0.5 text-[10px] font-semibold uppercase tracking-[0.06em] leading-4 text-base-content/48 font-mono tabular-nums',
          labelClassName,
        )}
      >
        {displayLabel}
      </span>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
        {renderMetric({ label: metricLabels.requests, value: displayRequests })}
        {renderMetric({ label: metricLabels.tokens, value: displayTokens, tooltip: tokenTooltip })}
        {renderMetric({ label: metricLabels.cost, value: displayCost })}
      </div>
      <div className="flex min-w-0 items-center gap-2">
        <span
          className={
            missing
              ? 'truncate whitespace-nowrap text-[11px] leading-4 text-base-content/55 font-mono tabular-nums'
              : 'truncate whitespace-nowrap text-[11px] leading-4 text-base-content/68 font-mono tabular-nums'
          }
        >
          {displayResetText}
        </span>
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <div className="h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-base-300/60">
            <div
              className={cn('h-full rounded-full bg-primary', accentClassName)}
              style={{ width: `${missing ? 0 : percent}%` }}
            />
          </div>
          <span
            className={
              missing
                ? 'w-[2.75rem] shrink-0 text-right text-[11px] font-semibold leading-4 text-base-content/55 font-mono tabular-nums'
                : 'w-[2.75rem] shrink-0 text-right text-[11px] font-semibold leading-4 text-base-content/78 font-mono tabular-nums'
            }
          >
            {missing ? WINDOW_PLACEHOLDER : `${Math.round(percent)}%`}
          </span>
        </div>
      </div>
    </div>
  )
}

function CompactTimestampLine({
  label,
  value,
  title,
}: {
  label: string
  value: string
  title?: string
}) {
  return (
    <div className="grid grid-cols-[max-content,minmax(0,1fr)] items-center gap-1" title={title ?? value}>
      <span className="truncate whitespace-nowrap text-[10px] font-semibold uppercase tracking-[0.06em] leading-4 text-base-content/48">
        {label}
      </span>
      <span className="truncate whitespace-nowrap text-[12px] leading-4 text-base-content/72 font-mono tabular-nums">
        {value}
      </span>
    </div>
  )
}

function formatWindowShortLabel(windowDurationMins?: number | null) {
  if (!Number.isFinite(windowDurationMins ?? NaN)) return null
  const minutes = Math.max(0, Math.round(windowDurationMins ?? 0))
  if (minutes === 300) return '5H'
  if (minutes === 10_080) return '7D'
  if (minutes % (60 * 24) === 0) return `${minutes / (60 * 24)}D`
  if (minutes % 60 === 0) return `${minutes / 60}H`
  return `${minutes}M`
}

function normalizeLabelResult(value: unknown) {
  return typeof value === 'string' || value == null ? value : null
}

function runActionDetailResolver(
  resolver: ActionDetailLabelResolver,
  value: UpstreamAccountSummary | string | null | undefined,
) {
  return normalizeLabelResult((resolver as (value: UpstreamAccountSummary | string | null | undefined) => unknown)(value))
}

function resolveActionSourceLabel(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps['labels'],
) {
  const fromItem = runActionDetailResolver(labels.actionSource, item)
  if (fromItem) return fromItem
  return runActionDetailResolver(labels.actionSource, item.lastActionSource)
}

function resolveActionReasonLabel(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps['labels'],
) {
  const fromItem = runActionDetailResolver(labels.actionReason, item)
  if (fromItem) return fromItem
  return runActionDetailResolver(labels.actionReason, item.lastActionReasonCode)
}

function buildLatestActionTitle(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps['labels'],
) {
  const message = item.lastActionReasonMessage ?? item.lastError
  const hasActionDetails =
    Boolean(item.lastAction || item.lastActionSource || item.lastActionReasonCode || item.lastActionAt || message) ||
    Number.isFinite(item.lastActionHttpStatus ?? NaN)
  if (!hasActionDetails) return null

  const action = labels.action(item.lastAction) ?? labels.unknown
  const source = resolveActionSourceLabel(item, labels) ?? labels.unknown
  const reason = resolveActionReasonLabel(item, labels) ?? labels.unknown
  const httpStatus = Number.isFinite(item.lastActionHttpStatus ?? NaN)
    ? `HTTP ${item.lastActionHttpStatus}`
    : labels.unavailable
  const occurredAt = formatDateTime(item.lastActionAt, labels.never)
  const parts = [
    `${labels.latestActionFieldAction}: ${action}`,
    `${labels.latestActionFieldSource}: ${source}`,
    `${labels.latestActionFieldReason}: ${reason}`,
    `${labels.latestActionFieldHttpStatus}: ${httpStatus}`,
    `${labels.latestActionFieldOccurredAt}: ${occurredAt}`,
  ]
  if (message) {
    parts.push(`${labels.latestActionFieldMessage}: ${message}`)
  }
  return parts.join(' · ')
}

function buildLatestActionSummary(
  item: UpstreamAccountSummary,
  labels: UpstreamAccountsTableProps['labels'],
) {
  const action = labels.action(item.lastAction)
  const source = resolveActionSourceLabel(item, labels)
  const reason = resolveActionReasonLabel(item, labels)
  const parts = [action ?? source, reason]
  if (Number.isFinite(item.lastActionHttpStatus ?? NaN)) {
    parts.push(`HTTP ${item.lastActionHttpStatus}`)
  }
  const compact = parts
    .filter((value): value is string => Boolean(value && value.trim()))
    .join(' · ')
  if (!compact) return formatDateTime(item.lastActionAt, labels.never)
  const timestamp = formatDateTime(item.lastActionAt, labels.never)
  return timestamp === labels.never ? compact : `${compact} · ${timestamp}`
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
  isLoading = false,
  error = null,
  loadingTitle,
  loadingDescription,
  errorTitle,
  retryLabel,
  onRetry,
  selectedId,
  selectedAccountIds,
  onSelect,
  onToggleSelected,
  onToggleSelectAllCurrentPage,
  emptyTitle,
  emptyDescription,
  labels,
}: UpstreamAccountsTableProps) {
  const showBlockingOverlay = isLoading && items.length > 0
  const containerRef = useRef<HTMLDivElement | null>(null)
  const blockingIndicatorRef = useRef<HTMLDivElement | null>(null)
  const [blockingIndicatorTop, setBlockingIndicatorTop] = useState<number | null>(null)

  useLayoutEffect(() => {
    if (!showBlockingOverlay) {
      setBlockingIndicatorTop(null)
      return
    }

    const updateBlockingIndicatorTop = () => {
      const container = containerRef.current
      const indicator = blockingIndicatorRef.current
      if (!container || !indicator) return

      const containerRect = container.getBoundingClientRect()
      const indicatorHeight = indicator.getBoundingClientRect().height || 0
      const viewportHeight = window.innerHeight || 0
      const padding = 24
      const visibleTop = Math.max(containerRect.top, 0)
      const visibleBottom = Math.min(containerRect.bottom, viewportHeight)
      const visibleCenter = visibleTop < visibleBottom
        ? visibleTop + (visibleBottom - visibleTop) / 2
        : Math.min(
            Math.max(containerRect.top + padding + indicatorHeight / 2, viewportHeight / 2),
            containerRect.bottom - padding - indicatorHeight / 2,
          )
      const minTop = padding
      const maxTop = Math.max(minTop, containerRect.height - indicatorHeight - padding)
      const nextTop = Math.min(
        Math.max(visibleCenter - containerRect.top - indicatorHeight / 2, minTop),
        maxTop,
      )

      setBlockingIndicatorTop((currentTop) =>
        currentTop != null && Math.abs(currentTop - nextTop) < 1 ? currentTop : nextTop,
      )
    }

    updateBlockingIndicatorTop()

    const handleViewportChange = () => {
      window.requestAnimationFrame(updateBlockingIndicatorTop)
    }

    window.addEventListener('scroll', handleViewportChange, { passive: true })
    window.addEventListener('resize', handleViewportChange)

    return () => {
      window.removeEventListener('scroll', handleViewportChange)
      window.removeEventListener('resize', handleViewportChange)
    }
  }, [showBlockingOverlay])

  if (isLoading && items.length === 0) {
    return (
      <div
        data-testid="upstream-accounts-table-loading"
        className="sticky top-6 z-10 flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/90 px-6 py-10 text-center shadow-sm backdrop-blur-sm"
        aria-live="polite"
      >
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Spinner className="h-6 w-6" />
        </div>
        <h3 className="text-lg font-semibold text-base-content">
          {loadingTitle ?? emptyTitle}
        </h3>
        {loadingDescription ? (
          <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
            {loadingDescription}
          </p>
        ) : null}
      </div>
    )
  }

  if (error && items.length === 0) {
    return (
      <div
        data-testid="upstream-accounts-table-error"
        className="sticky top-6 z-10 flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-error/30 bg-error/10 px-6 py-10 text-center shadow-sm backdrop-blur-sm"
        aria-live="polite"
      >
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-error/10 text-error">
          <AppIcon name="alert-circle-outline" className="h-7 w-7" aria-hidden />
        </div>
        <h3 className="text-lg font-semibold text-base-content">
          {errorTitle ?? emptyTitle}
        </h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-base-content/70">
          {error}
        </p>
        {onRetry && retryLabel ? (
          <Button type="button" variant="secondary" className="mt-4" onClick={onRetry}>
            <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
            {retryLabel}
          </Button>
        ) : null}
      </div>
    )
  }

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

  const currentPageSelectedCount = items.filter((item) => selectedAccountIds.has(item.id)).length
  const allCurrentPageSelected = items.length > 0 && currentPageSelectedCount === items.length
  const partiallySelected =
    currentPageSelectedCount > 0 && currentPageSelectedCount < items.length

  return (
    <div
      ref={containerRef}
      className="relative overflow-x-auto rounded-[1.35rem] border border-base-300/80 bg-base-100/72 md:overflow-x-visible"
      aria-busy={showBlockingOverlay ? 'true' : undefined}
    >
      <table
        className={cn(
          'min-w-[54rem] w-full table-auto border-collapse md:min-w-0 md:table-fixed',
          showBlockingOverlay && 'pointer-events-none select-none opacity-45',
        )}
      >
        <colgroup>
          <col className="w-[3rem]" />
          <col className="w-[38%]" />
          <col className="w-[16%]" />
          <col className="w-[42%]" />
          <col className="w-[4%]" />
        </colgroup>
        <thead>
          <tr className="border-b border-base-300/80 bg-base-100/86 text-left">
            <th className="px-3 py-2.5 text-center">
              <SelectAllCheckbox
                checked={allCurrentPageSelected}
                indeterminate={partiallySelected}
                ariaLabel={labels.selectPage}
                onChange={onToggleSelectAllCurrentPage}
              />
            </th>
            <th className="px-4 py-2.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              {labels.account}
            </th>
            <th className="pl-1 pr-3 py-2.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              {labels.sync}
            </th>
            <th className="pl-1 pr-3 py-2.5 text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
              {labels.windows}
            </th>
            <th className="w-12 pl-1 pr-3 py-2.5" aria-hidden />
          </tr>
        </thead>
        <tbody>
          {items.map((item, index) => {
            const primaryWindowMissing = item.primaryWindow == null
            const secondaryWindowMissing = item.secondaryWindow == null
            const primary = windowPercent(item.primaryWindow?.usedPercent)
            const secondary = windowPercent(item.secondaryWindow?.usedPercent)
            const primaryResetText = item.primaryWindow?.resetsAt
              ? `${labels.nextResetCompact ?? labels.nextReset} ${formatDateTime(item.primaryWindow.resetsAt)}`
              : undefined
            const secondaryResetText = item.secondaryWindow?.resetsAt
              ? `${labels.nextResetCompact ?? labels.nextReset} ${formatDateTime(item.secondaryWindow.resetsAt)}`
              : undefined
            const primaryLabel =
              formatWindowShortLabel(item.primaryWindow?.windowDurationMins) ?? labels.primaryShort.toUpperCase()
            const secondaryLabel =
              formatWindowShortLabel(item.secondaryWindow?.windowDurationMins) ?? labels.secondaryShort.toUpperCase()
            const primaryWindowUnexpected =
              item.primaryWindow != null &&
              Number.isFinite(item.primaryWindow.windowDurationMins) &&
              Math.round(item.primaryWindow.windowDurationMins) !== 300
            const secondaryWindowUnexpected =
              item.secondaryWindow != null &&
              Number.isFinite(item.secondaryWindow.windowDurationMins) &&
              Math.round(item.secondaryWindow.windowDurationMins) !== 10_080
            const selected = item.id === selectedId
            const enableStatus = accountEnableStatus(item)
            const healthStatus = accountHealthStatus(item)
            const syncState = accountSyncState(item)
            const availabilityBadge = resolveAvailabilityBadge(item, labels)
            const routingBlockMessage = item.routingBlockReasonMessage?.trim() || null
            const latestActionTitle = buildLatestActionTitle(item, labels)
            const healthBadgeTitle =
              healthStatus !== 'normal'
                ? item.lastActionReasonMessage ?? item.lastError ?? latestActionTitle
                : undefined
            const primaryWindowTitle = [item.primaryWindow?.limitText, primaryResetText].filter(Boolean).join(' · ') || undefined
            const secondaryWindowTitle =
              [item.secondaryWindow?.limitText, secondaryResetText].filter(Boolean).join(' · ') || undefined
            const planBadge = upstreamPlanBadgeRecipe(item.planType)
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
                <td className="px-3 py-3 align-middle text-center">
                  <input
                    type="checkbox"
                    className="h-4 w-4 cursor-pointer rounded border-base-300/90 bg-base-100 accent-primary"
                    aria-label={labels.selectRow(item.displayName)}
                    checked={selectedAccountIds.has(item.id)}
                    onChange={(event) => onToggleSelected(item.id, event.target.checked)}
                    onClick={(event) => event.stopPropagation()}
                    onKeyDown={(event) => event.stopPropagation()}
                  />
                </td>
                <td className="px-4 py-3">
                  <div className="min-w-0">
                    <p
                      className="truncate whitespace-nowrap text-[14px] font-semibold leading-5 text-base-content"
                      title={item.displayName}
                    >
                      {item.displayName}
                    </p>
                    <div className="mt-2 min-w-0 space-y-1.5">
                      <div className="flex min-w-0 flex-wrap items-center gap-1">
                        {item.isMother ? (
                          <div className="shrink-0">
                            <MotherAccountBadge label={labels.mother} />
                          </div>
                        ) : null}
                        {item.duplicateInfo
                          ? compactBadge(labels.duplicate, 'warning')
                          : null}
                        {compactBadge(labels.enableStatus(enableStatus), enableBadgeVariant(enableStatus))}
                        {availabilityBadge
                          ? compactBadge(availabilityBadge.label, availabilityBadge.variant)
                          : null}
                        {syncState === 'syncing'
                          ? compactBadge(labels.syncState(syncState), syncBadgeVariant(syncState))
                          : null}
                        {healthStatus !== 'normal'
                          ? compactBadge(labels.healthStatus(healthStatus), healthBadgeVariant(healthStatus), {
                            title: healthBadgeTitle ?? undefined,
                          })
                          : null}
                        {compactBadge(kindLabel(item, labels), 'secondary')}
                        {item.compactSupport?.status === 'unsupported' && labels.compactSupport?.(item)
                          ? compactBadge(
                            labels.compactSupport(item) ?? '',
                            'warning',
                            {
                              title: labels.compactSupportHint?.(item) ?? undefined,
                            },
                          )
                          : null}
                        {item.planType && planBadge
                          ? compactBadge(item.planType, planBadge.variant, {
                            className: planBadge.className,
                            dataPlan: planBadge.dataPlan,
                            title: item.planType,
                          })
                          : item.planType
                            ? compactBadge(item.planType, 'accent', { title: item.planType })
                          : null}
                      </div>
                      <div className="flex min-w-0 flex-wrap items-center gap-1">
                        <div className="flex min-w-0 flex-wrap items-center gap-1">
                          {renderTagBadges(item.tags)}
                        </div>
                        {renderTagOverflowBadge(labels, item.tags)}
                      </div>
                    </div>
                  </div>
                </td>
                <td className="pl-1 pr-3 py-3 align-middle">
                  <div className="space-y-1">
                    <CompactTimestampLine
                      label={labels.lastSuccess}
                      value={formatDateTime(item.lastSuccessfulSyncAt, labels.never)}
                    />
                    <CompactTimestampLine
                      label={labels.lastCall}
                      value={formatDateTime(item.lastActivityAt, labels.never)}
                    />
                    {routingBlockMessage ? (
                      <CompactTimestampLine
                        label={labels.routingBlock}
                        value={routingBlockMessage}
                        title={routingBlockMessage}
                      />
                    ) : null}
                    <CompactTimestampLine
                      label={labels.latestAction}
                      value={buildLatestActionSummary(item, labels)}
                      title={latestActionTitle ?? undefined}
                    />
                  </div>
                </td>
                <td className="pl-1 pr-3 py-3 align-middle">
                  <div className="space-y-1.5">
                    <CompactWindowLine
                      window={item.primaryWindow}
                      label={primaryLabel}
                      percent={primary}
                      resetText={primaryResetText}
                      metricLabels={{
                        requests: labels.requestsMetric,
                        tokens: labels.tokensMetric,
                        cost: labels.costMetric,
                        inputTokens: labels.inputTokensMetric,
                        outputTokens: labels.outputTokensMetric,
                        cacheInputTokens: labels.cacheInputTokensMetric,
                      }}
                      missing={primaryWindowMissing}
                      title={primaryWindowTitle}
                      labelClassName={primaryWindowUnexpected ? 'text-warning/78' : undefined}
                    />
                    <CompactWindowLine
                      window={item.secondaryWindow}
                      label={secondaryLabel}
                      percent={secondary}
                      resetText={secondaryResetText}
                      metricLabels={{
                        requests: labels.requestsMetric,
                        tokens: labels.tokensMetric,
                        cost: labels.costMetric,
                        inputTokens: labels.inputTokensMetric,
                        outputTokens: labels.outputTokensMetric,
                        cacheInputTokens: labels.cacheInputTokensMetric,
                      }}
                      missing={secondaryWindowMissing}
                      hideLabelWhenMissing={item.localLimits?.secondaryLimit === null}
                      accentClassName="bg-secondary"
                      title={secondaryWindowTitle}
                      labelClassName={secondaryWindowUnexpected ? 'text-warning/78' : undefined}
                    />
                  </div>
                </td>
                <td className="pl-1 pr-3 py-3 text-right align-middle">
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
      {showBlockingOverlay ? (
        <div
          data-testid="upstream-accounts-table-loading-overlay"
          className="pointer-events-none absolute inset-0 z-10 rounded-[1.35rem] bg-base-100/38 backdrop-blur-[3px]"
        />
      ) : null}
      {showBlockingOverlay ? (
        <div
          className="pointer-events-none absolute inset-x-0 z-20 flex justify-center px-4"
          style={{ top: blockingIndicatorTop != null ? `${blockingIndicatorTop}px` : '24px' }}
        >
          <div
            ref={blockingIndicatorRef}
            data-testid="upstream-accounts-table-loading-indicator"
            role="status"
            aria-live="polite"
            className="flex max-w-[min(calc(100%-2rem),32rem)] items-center gap-3 rounded-[1.75rem] border border-base-100/70 bg-base-100/70 px-5 py-4 text-sm text-base-content shadow-[0_18px_60px_rgba(15,23,42,0.16)] ring-1 ring-base-100/70 backdrop-blur-xl"
          >
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full bg-primary/12 text-primary">
              <Spinner className="h-5 w-5" />
            </div>
            <div className="min-w-0">
              <div className="font-semibold text-base-content">
                {loadingTitle ?? emptyTitle}
              </div>
              {loadingDescription ? (
                <div className="mt-1 text-sm leading-6 text-base-content/70">
                  {loadingDescription}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
}
