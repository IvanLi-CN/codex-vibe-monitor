import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { AppIcon } from './AppIcon'
import { Button } from './ui/button'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'
import { cn } from '../lib/utils'
import { upstreamPlanBadgeRecipe } from '../lib/upstreamAccountBadges'
import type { UpstreamAccountSummary } from '../lib/api'
import {
  CompactTimestampLine,
  CompactWindowLine,
  accountEnableStatus,
  accountHealthStatus,
  accountSyncState,
  buildLatestActionSummary,
  buildLatestActionTitle,
  compactBadge,
  enableBadgeVariant,
  formatDateTime,
  formatWindowShortLabel,
  handleRowKeyDown,
  healthBadgeVariant,
  kindLabel,
  renderTagBadges,
  renderTagOverflowBadge,
  resolveAvailabilityBadge,
  resolveCurrentForwardProxyBadgeLabel,
  resolveCurrentForwardProxyBadgeVariant,
  syncBadgeVariant,
  type UpstreamAccountsTableLabels,
  windowPercent,
} from './UpstreamAccountsTable'
import { MotherAccountBadge } from './MotherAccountToggle'

const GROUP_CARD_ESTIMATE_PX = 420
const GROUP_MEMBER_ROW_ESTIMATE_PX = 104
const GROUP_MEMBER_ROW_GAP_PX = 8
const GROUP_MEMBER_MIN_VISIBLE_ROWS = 2
const GROUP_MEMBER_MAX_VISIBLE_ROWS = 10
const GROUP_MEMBER_GRID_CARD_ESTIMATE_PX = 144
const GROUP_MEMBER_GRID_GAP_PX = 12
const GROUP_MEMBER_GRID_MAX_VISIBLE_ROWS = 5
const GROUP_MEMBER_GRID_TWO_COLUMN_BREAKPOINT_PX = 560
const GROUP_MEMBER_GRID_THREE_COLUMN_BREAKPOINT_PX = 960

type GroupPlanCount = {
  key: string
  label: string
  count: number
}

export interface UpstreamAccountsGroupedRosterGroup {
  id: string
  groupName: string | null
  displayName: string
  items: UpstreamAccountSummary[]
  note?: string | null
  boundProxyLabels?: string[]
  concurrencyLimit?: number | null
  nodeShuntEnabled?: boolean
  planCounts: GroupPlanCount[]
}

interface UpstreamAccountsGroupedRosterProps {
  groups: UpstreamAccountsGroupedRosterGroup[]
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
  onToggleSelected?: (accountId: number, checked: boolean) => void
  onToggleSelectAllVisible?: (checked: boolean) => void
  emptyTitle: string
  emptyDescription: string
  labels: UpstreamAccountsTableLabels
  memberLayout?: 'list' | 'grid'
  selectionMode?: 'multi' | 'none'
  groupLabels: {
    count: (count: number) => string
    concurrency: (value: number) => string
    exclusiveNode: string
    selectVisible: string
    infoTitle: string
    noteLabel: string
    noteEmpty: string
    proxiesLabel: string
    proxiesEmpty: string
  }
}

function groupPlanBadgeRecipe(planKey: string) {
  if (planKey === 'api') {
    return {
      variant: 'info' as const,
      className: undefined,
      dataPlan: undefined,
    }
  }
  return upstreamPlanBadgeRecipe(planKey)
}

function GroupSummaryPanel({
  group,
  groupLabels,
  compact = false,
}: {
  group: UpstreamAccountsGroupedRosterGroup
  groupLabels: UpstreamAccountsGroupedRosterProps['groupLabels']
  compact?: boolean
}) {
  return (
    <div
      className={cn(
        'flex flex-col gap-2 xl:pr-3.5',
        !compact && 'xl:border-r xl:border-base-300/65',
      )}
    >
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
        <h3
          className="min-w-0 text-[16px] font-semibold leading-5 text-base-content"
          title={group.displayName}
        >
          <span className="block truncate">{group.displayName}</span>
        </h3>
        <span className="shrink-0 text-[11px] font-medium leading-4 text-base-content/46">
          {groupLabels.count(group.items.length)}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        {group.planCounts.map((plan) => {
          const recipe = groupPlanBadgeRecipe(plan.key)
          const content = `${plan.label} ${plan.count}`
          return (
            <Badge
              key={plan.key}
              variant={recipe?.variant ?? 'secondary'}
              className={cn(
                'shrink-0 whitespace-nowrap px-2 py-px text-[11px] font-medium leading-4',
                recipe?.className,
              )}
              data-plan={recipe?.dataPlan}
            >
              {content}
            </Badge>
          )
        })}
        {typeof group.concurrencyLimit === 'number' && group.concurrencyLimit > 0 ? (
          <Badge variant="secondary" className="px-2 py-px text-[11px] font-medium leading-4">
            {groupLabels.concurrency(group.concurrencyLimit)}
          </Badge>
        ) : null}
        {group.nodeShuntEnabled ? (
          <Badge variant="info" className="px-2 py-px text-[11px] font-medium leading-4">
            {groupLabels.exclusiveNode}
          </Badge>
        ) : null}
      </div>

      <div className="flex flex-wrap items-center gap-1.5 text-[12px] leading-5 text-base-content/54">
        <span className="shrink-0 font-medium uppercase tracking-[0.12em] text-base-content/42">
          {groupLabels.proxiesLabel}
        </span>
        <div className="flex min-w-0 flex-wrap items-center gap-1.5">
          {Array.isArray(group.boundProxyLabels) && group.boundProxyLabels.length > 0 ? (
            group.boundProxyLabels.map((label) => (
              <Badge
                key={label}
                variant="secondary"
                className="max-w-full px-2 py-px text-[11px] font-medium leading-4"
                title={label}
              >
                <span className="truncate">{label}</span>
              </Badge>
            ))
          ) : (
            <span className="text-[12px] leading-5 text-base-content/58">
              {groupLabels.proxiesEmpty}
            </span>
          )}
        </div>
      </div>
    </div>
  )
}

function memberViewportHeightForRows(rowCount: number) {
  return (
    rowCount * GROUP_MEMBER_ROW_ESTIMATE_PX +
    Math.max(0, rowCount - 1) * GROUP_MEMBER_ROW_GAP_PX
  )
}

const GROUP_MEMBER_MAX_HEIGHT_PX = memberViewportHeightForRows(
  GROUP_MEMBER_MAX_VISIBLE_ROWS,
)

function shouldVirtualizeGroupMembers(count: number) {
  return count > GROUP_MEMBER_MAX_VISIBLE_ROWS
}

function memberGridViewportHeightForRows(rowCount: number) {
  return (
    rowCount * GROUP_MEMBER_GRID_CARD_ESTIMATE_PX +
    Math.max(0, rowCount - 1) * GROUP_MEMBER_GRID_GAP_PX
  )
}

const GROUP_MEMBER_GRID_MAX_HEIGHT_PX = memberGridViewportHeightForRows(
  GROUP_MEMBER_GRID_MAX_VISIBLE_ROWS,
)

function resolveGridColumnCount(width: number) {
  if (width >= GROUP_MEMBER_GRID_THREE_COLUMN_BREAKPOINT_PX) return 3
  if (width >= GROUP_MEMBER_GRID_TWO_COLUMN_BREAKPOINT_PX) return 2
  return 1
}

function shouldShowPlanBadge(planType?: string | null) {
  const normalized = planType?.trim().toLowerCase()
  return Boolean(normalized && normalized !== 'local')
}

function GroupMemberRow({
  item,
  selectedId,
  selectedAccountIds,
  onSelect,
  onToggleSelected,
  labels,
  selectionMode = 'multi',
}: {
  item: UpstreamAccountSummary
  selectedId: number | null
  selectedAccountIds: Set<number>
  onSelect: (accountId: number) => void
  onToggleSelected?: (accountId: number, checked: boolean) => void
  labels: UpstreamAccountsTableLabels
  selectionMode?: 'multi' | 'none'
}) {
  const selected = item.id === selectedId
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
  const showPlanBadge = shouldShowPlanBadge(item.planType)
  const planBadge = showPlanBadge ? upstreamPlanBadgeRecipe(item.planType) : null

  const selectionEnabled = selectionMode === 'multi' && typeof onToggleSelected === 'function'

  return (
    <div
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={() => onSelect(item.id)}
      onKeyDown={(event) => handleRowKeyDown(event as KeyboardEvent<HTMLTableRowElement>, item.id, onSelect)}
      className={cn(
        'rounded-[0.85rem] px-2.5 py-1.5 outline-none transition-colors hover:bg-base-200/55 focus-visible:bg-base-200/55',
        selected && 'bg-primary/8 ring-1 ring-primary/18',
      )}
    >
      <div className="flex items-start gap-3">
        {selectionEnabled ? (
          <input
            type="checkbox"
            className="mt-1 h-4 w-4 cursor-pointer rounded border-base-300/90 bg-base-100 accent-primary"
            aria-label={labels.selectRow(item.displayName)}
            checked={selectedAccountIds.has(item.id)}
            onChange={(event) => onToggleSelected(item.id, event.target.checked)}
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          />
        ) : null}
        <div className="grid min-w-0 flex-1 gap-3 xl:grid-cols-[minmax(0,1.35fr)_minmax(12rem,0.7fr)_minmax(18rem,1fr)_auto]">
          <div className="min-w-0">
            <p
              className="truncate whitespace-nowrap text-[14px] font-semibold leading-5 text-base-content"
              title={item.displayName}
            >
              {item.displayName}
            </p>
            <div className="mt-1.5 min-w-0 space-y-1.5">
              <div className="flex min-w-0 flex-wrap items-center gap-1">
                {item.isMother ? (
                  <div className="shrink-0">
                    <MotherAccountBadge label={labels.mother} />
                  </div>
                ) : null}
                {item.duplicateInfo ? compactBadge(labels.duplicate, 'warning') : null}
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
                {showPlanBadge && item.planType && planBadge
                  ? compactBadge(item.planType, planBadge.variant, {
                      className: planBadge.className,
                      dataPlan: planBadge.dataPlan,
                      title: item.planType,
                    })
                  : showPlanBadge && item.planType
                    ? compactBadge(item.planType, 'accent', { title: item.planType })
                    : null}
                {compactBadge(
                  resolveCurrentForwardProxyBadgeLabel(item, labels),
                  resolveCurrentForwardProxyBadgeVariant(item),
                  { title: resolveCurrentForwardProxyBadgeLabel(item, labels) },
                )}
              </div>
              <div className="flex min-w-0 flex-wrap items-center gap-1">
                <div className="flex min-w-0 flex-wrap items-center gap-1">
                  {renderTagBadges(item.tags)}
                </div>
                {renderTagOverflowBadge(labels, item.tags)}
              </div>
            </div>
          </div>

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

          <div className="flex items-center justify-end xl:pr-1">
            <AppIcon
              name={selected ? 'chevron-right-circle' : 'chevron-right'}
              className={cn('h-5 w-5', selected ? 'text-primary' : 'text-base-content/35')}
              aria-hidden
            />
          </div>
        </div>
      </div>
    </div>
  )
}

function GroupMemberGridCard({
  item,
  selectedId,
  onSelect,
  labels,
}: {
  item: UpstreamAccountSummary
  selectedId: number | null
  onSelect: (accountId: number) => void
  labels: UpstreamAccountsTableLabels
}) {
  const selected = item.id === selectedId
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
  const showPlanBadge = shouldShowPlanBadge(item.planType)
  const planBadge = showPlanBadge ? upstreamPlanBadgeRecipe(item.planType) : null

  return (
    <div
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={() => onSelect(item.id)}
      onKeyDown={(event) =>
        handleRowKeyDown(
          event as unknown as KeyboardEvent<HTMLTableRowElement>,
          item.id,
          onSelect,
        )
      }
      className={cn(
        'rounded-[0.95rem] border border-base-300/55 bg-base-100/56 p-3 outline-none transition-colors hover:bg-base-100/86 focus-visible:bg-base-100/86',
        selected && 'border-primary/30 bg-primary/8 shadow-[0_0_0_1px_rgba(59,130,246,0.08)]',
      )}
      data-testid="upstream-accounts-group-grid-card"
    >
      <div className="min-w-0">
        <p className="truncate text-[14px] font-semibold leading-5 text-base-content" title={item.displayName}>
          {item.displayName}
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-1">
          {compactBadge(kindLabel(item, labels), 'secondary')}
          {showPlanBadge && item.planType && planBadge
            ? compactBadge(item.planType, planBadge.variant, {
                className: planBadge.className,
                dataPlan: planBadge.dataPlan,
                title: item.planType,
              })
            : showPlanBadge && item.planType
              ? compactBadge(item.planType, 'accent', { title: item.planType })
              : null}
        </div>
      </div>
      <div className="mt-3 space-y-1.5">
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
          missing={item.primaryWindow == null}
          title={[item.primaryWindow?.limitText, primaryResetText].filter(Boolean).join(' · ') || undefined}
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
          missing={item.secondaryWindow == null}
          hideLabelWhenMissing={item.localLimits?.secondaryLimit === null}
          accentClassName="bg-secondary"
          title={[item.secondaryWindow?.limitText, secondaryResetText].filter(Boolean).join(' · ') || undefined}
        />
      </div>
    </div>
  )
}

function GroupMembersVirtualList({
  items,
  selectedId,
  selectedAccountIds,
  onSelect,
  onToggleSelected,
  labels,
  memberLayout = 'list',
  selectionMode = 'multi',
}: {
  items: UpstreamAccountSummary[]
  selectedId: number | null
  selectedAccountIds: Set<number>
  onSelect: (accountId: number) => void
  onToggleSelected?: (accountId: number, checked: boolean) => void
  labels: UpstreamAccountsTableLabels
  memberLayout?: 'list' | 'grid'
  selectionMode?: 'multi' | 'none'
}) {
  const isGridLayout = memberLayout === 'grid'
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const [gridColumnCount, setGridColumnCount] = useState(() =>
    resolveGridColumnCount(typeof window === 'undefined' ? 0 : window.innerWidth),
  )

  useEffect(() => {
    if (!isGridLayout) return
    const element = scrollRef.current
    if (!element) return

    const updateColumnCount = (width?: number) => {
      const fallbackWidth =
        width ??
        element.getBoundingClientRect().width ??
        (typeof window === 'undefined' ? 0 : window.innerWidth)
      const next = resolveGridColumnCount(fallbackWidth)
      setGridColumnCount((current) => (current === next ? current : next))
    }

    updateColumnCount()

    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver((entries) => {
      updateColumnCount(entries[0]?.contentRect.width)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [isGridLayout])

  const safeGridColumnCount = Math.max(1, gridColumnCount)
  const gridRowCount = Math.ceil(items.length / safeGridColumnCount)
  const gridVirtualized = isGridLayout && gridRowCount > GROUP_MEMBER_GRID_MAX_VISIBLE_ROWS
  const listVirtualized = !isGridLayout && shouldVirtualizeGroupMembers(items.length)
  const viewportHeight = listVirtualized ? GROUP_MEMBER_MAX_HEIGHT_PX : undefined
  const minimumVisibleRows = Math.max(1, Math.min(items.length, GROUP_MEMBER_MIN_VISIBLE_ROWS))
  const minimumHeight = memberViewportHeightForRows(minimumVisibleRows)
  const rowVirtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => (listVirtualized ? scrollRef.current : null),
    estimateSize: () => GROUP_MEMBER_ROW_ESTIMATE_PX,
    overscan: 4,
    gap: GROUP_MEMBER_ROW_GAP_PX,
    enabled: listVirtualized,
  })
  const gridRowVirtualizer = useVirtualizer({
    count: gridRowCount,
    getScrollElement: () => (gridVirtualized ? scrollRef.current : null),
    estimateSize: () => GROUP_MEMBER_GRID_CARD_ESTIMATE_PX,
    overscan: 2,
    gap: GROUP_MEMBER_GRID_GAP_PX,
    enabled: gridVirtualized,
  })

  if (isGridLayout) {
    return (
      <div
        ref={scrollRef}
        className="self-start min-w-0 overflow-auto py-1"
        style={{
          ...(gridVirtualized ? { height: `${GROUP_MEMBER_GRID_MAX_HEIGHT_PX}px` } : null),
        }}
        data-testid="upstream-accounts-group-members-grid"
      >
        {gridVirtualized ? (
          <div
            className="relative w-full"
            style={{ height: `${gridRowVirtualizer.getTotalSize()}px` }}
          >
            {gridRowVirtualizer.getVirtualItems().map((virtualRow) => {
              const startIndex = virtualRow.index * safeGridColumnCount
              const rowItems = items.slice(startIndex, startIndex + safeGridColumnCount)
              return (
                <div
                  key={`grid-row-${virtualRow.index}`}
                  ref={gridRowVirtualizer.measureElement}
                  data-index={virtualRow.index}
                  className="absolute left-0 top-0 w-full"
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  <div
                    className="grid gap-3"
                    style={{
                      gridTemplateColumns: `repeat(${safeGridColumnCount}, minmax(0, 1fr))`,
                    }}
                  >
                    {rowItems.map((item) => (
                      <GroupMemberGridCard
                        key={item.id}
                        item={item}
                        selectedId={selectedId}
                        onSelect={onSelect}
                        labels={labels}
                      />
                    ))}
                  </div>
                </div>
              )
            })}
          </div>
        ) : (
          <div
            className="grid gap-3"
            style={{
              gridTemplateColumns: `repeat(${safeGridColumnCount}, minmax(0, 1fr))`,
            }}
          >
            {items.map((item) => (
              <GroupMemberGridCard
                key={item.id}
                item={item}
                selectedId={selectedId}
                onSelect={onSelect}
                labels={labels}
              />
            ))}
          </div>
        )}
      </div>
    )
  }

  return (
    <div
      ref={scrollRef}
      className={cn(
        'min-w-0',
        listVirtualized ? 'overflow-auto' : 'overflow-hidden',
      )}
      style={{
        minHeight: `${minimumHeight}px`,
        ...(viewportHeight != null ? { height: `${viewportHeight}px` } : null),
      }}
      data-testid="upstream-accounts-group-members"
    >
      {listVirtualized ? (
        <div
          className="relative w-full"
          style={{ height: `${rowVirtualizer.getTotalSize()}px` }}
        >
          {rowVirtualizer.getVirtualItems().map((virtualRow) => {
            const item = items[virtualRow.index]
            return (
              <div
                key={item.id}
                ref={rowVirtualizer.measureElement}
                data-index={virtualRow.index}
                data-testid="upstream-accounts-group-row"
                className="absolute left-0 top-0 w-full"
                style={{ transform: `translateY(${virtualRow.start}px)` }}
              >
                <div
                  className={cn(
                    virtualRow.index === 0
                      ? 'pb-2'
                      : 'border-t border-base-300/60 py-2',
                  )}
                >
                  <GroupMemberRow
                    item={item}
                    selectedId={selectedId}
                    selectedAccountIds={selectedAccountIds}
                    onSelect={onSelect}
                    onToggleSelected={onToggleSelected}
                    labels={labels}
                    selectionMode={selectionMode}
                  />
                </div>
              </div>
            )
          })}
        </div>
      ) : (
        <div className="divide-y divide-base-300/60">
          {items.map((item) => (
            <div
              key={item.id}
              data-testid="upstream-accounts-group-row"
              className="py-2 first:pt-0 last:pb-0"
            >
              <GroupMemberRow
                item={item}
                selectedId={selectedId}
                selectedAccountIds={selectedAccountIds}
                onSelect={onSelect}
                onToggleSelected={onToggleSelected}
                labels={labels}
                selectionMode={selectionMode}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

export function UpstreamAccountsGroupedRoster({
  groups,
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
  onToggleSelectAllVisible,
  emptyTitle,
  emptyDescription,
  labels,
  memberLayout = 'list',
  selectionMode = 'multi',
  groupLabels,
}: UpstreamAccountsGroupedRosterProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null)
  const selectAllRef = useRef<HTMLInputElement | null>(null)
  const groupVirtualizer = useVirtualizer({
    count: groups.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => GROUP_CARD_ESTIMATE_PX,
    overscan: 3,
    gap: 16,
  })
  const selectionEnabled = selectionMode === 'multi' && typeof onToggleSelected === 'function'
  const totalVisibleCount = groups.reduce((sum, group) => sum + group.items.length, 0)
  const selectedVisibleCount = groups.reduce(
    (sum, group) =>
      sum + group.items.filter((item) => selectedAccountIds.has(item.id)).length,
    0,
  )
  const allVisibleSelected = totalVisibleCount > 0 && selectedVisibleCount === totalVisibleCount
  const partiallySelected =
    selectedVisibleCount > 0 && selectedVisibleCount < totalVisibleCount

  useEffect(() => {
    if (selectAllRef.current) {
      selectAllRef.current.indeterminate = partiallySelected
    }
  }, [partiallySelected])

  if (isLoading && groups.length === 0) {
    return (
      <div
        data-testid="upstream-accounts-grouped-loading"
        className="sticky top-6 z-10 flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/90 px-6 py-10 text-center shadow-sm backdrop-blur-sm"
        aria-live="polite"
      >
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <Spinner className="h-6 w-6" />
        </div>
        <h3 className="text-lg font-semibold text-base-content">{loadingTitle ?? emptyTitle}</h3>
        {loadingDescription ? (
          <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">{loadingDescription}</p>
        ) : null}
      </div>
    )
  }

  if (error && groups.length === 0) {
    return (
      <div
        data-testid="upstream-accounts-grouped-error"
        className="sticky top-6 z-10 flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-error/30 bg-error/10 px-6 py-10 text-center shadow-sm backdrop-blur-sm"
        aria-live="polite"
      >
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-error/10 text-error">
          <AppIcon name="alert-circle-outline" className="h-7 w-7" aria-hidden />
        </div>
        <h3 className="text-lg font-semibold text-base-content">{errorTitle ?? emptyTitle}</h3>
        <p className="mt-2 max-w-md text-sm leading-6 text-base-content/70">{error}</p>
        {onRetry && retryLabel ? (
          <Button type="button" variant="secondary" className="mt-4" onClick={onRetry}>
            <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
            {retryLabel}
          </Button>
        ) : null}
      </div>
    )
  }

  if (groups.length === 0) {
    return (
      <div className="flex min-h-[16rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 py-10 text-center">
        <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
          <AppIcon name="server-network-outline" className="h-7 w-7" aria-hidden />
        </div>
        <h3 className="text-lg font-semibold text-base-content">{emptyTitle}</h3>
        <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">{emptyDescription}</p>
      </div>
    )
  }

  return (
    <div
      ref={scrollRef}
      className={cn(
        "relative max-h-[960px] overflow-auto",
        isLoading && "pointer-events-none select-none opacity-60",
      )}
      data-testid="upstream-accounts-grouped-roster"
      aria-busy={isLoading ? 'true' : undefined}
    >
      {selectionEnabled && onToggleSelectAllVisible ? (
        <div className="sticky top-0 z-10 mb-4 flex items-center justify-between border-b border-base-300/70 bg-base-100/94 px-2 pb-3 pt-1 backdrop-blur">
          <label className="flex items-center gap-2 text-sm font-medium text-base-content">
            <input
              ref={selectAllRef}
              type="checkbox"
              className="h-4 w-4 cursor-pointer rounded border-base-300/90 bg-base-100 accent-primary"
              aria-label={groupLabels.selectVisible}
              checked={allVisibleSelected}
              onChange={(event) => onToggleSelectAllVisible(event.target.checked)}
            />
            <span>{groupLabels.selectVisible}</span>
          </label>
          <span className="text-xs text-base-content/55">
            {groupLabels.count(totalVisibleCount)}
          </span>
        </div>
      ) : null}
      <div className="relative w-full" style={{ height: `${groupVirtualizer.getTotalSize()}px` }}>
        {groupVirtualizer.getVirtualItems().map((virtualGroup) => {
          const group = groups[virtualGroup.index]
          return (
            <div
              key={group.id}
              ref={groupVirtualizer.measureElement}
              data-index={virtualGroup.index}
              data-testid="upstream-accounts-group-card"
              className="absolute left-0 top-0 w-full"
              style={{ transform: `translateY(${virtualGroup.start}px)` }}
            >
              <article className="rounded-[1.1rem] border border-base-300/65 bg-base-100/76 px-3.5 py-3 shadow-[0_8px_24px_rgba(2,6,23,0.06)]">
                <div className="grid items-start gap-3.5 xl:grid-cols-[12.5rem_minmax(0,1fr)]">
                  <GroupSummaryPanel
                    group={group}
                    groupLabels={groupLabels}
                    compact={memberLayout === 'grid'}
                  />

                  <GroupMembersVirtualList
                    items={group.items}
                    selectedId={selectedId}
                    selectedAccountIds={selectedAccountIds}
                    onSelect={onSelect}
                    onToggleSelected={onToggleSelected}
                    labels={labels}
                    memberLayout={memberLayout}
                    selectionMode={selectionMode}
                  />
                </div>
              </article>
            </div>
          )
        })}
      </div>
    </div>
  )
}
