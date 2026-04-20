import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from 'react'
import { useWindowVirtualizer } from '@tanstack/react-virtual'
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

const GROUP_CARD_VERTICAL_GAP_PX = 16
const GROUP_SUMMARY_ESTIMATE_PX = 176
const GROUP_MEMBER_ROW_ESTIMATE_PX = 104
const GROUP_MEMBER_ROW_GAP_PX = 8
const GROUP_MEMBER_GRID_CARD_ESTIMATE_PX = 144
const GROUP_MEMBER_GRID_GAP_PX = 12
const GROUP_MEMBER_GRID_TWO_COLUMN_BREAKPOINT_PX = 960
const GROUP_MEMBER_GRID_THREE_COLUMN_BREAKPOINT_PX = 1040
const GROUP_CARD_HORIZONTAL_PADDING_PX = 28
const GROUP_SUMMARY_COLUMN_WIDTH_PX = 200
const GROUP_SUMMARY_GRID_BREAKPOINT_PX = 1280
const GROUP_SUMMARY_GRID_GAP_PX = 14

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
  hasCustomSettings?: boolean
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
  canEditGroupSettings?: boolean
  onEditGroupSettings?: (group: UpstreamAccountsGroupedRosterGroup) => void
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
    settingsLabel: string
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

function resolveGridColumnCount(width: number) {
  if (width >= GROUP_MEMBER_GRID_THREE_COLUMN_BREAKPOINT_PX) return 3
  if (width >= GROUP_MEMBER_GRID_TWO_COLUMN_BREAKPOINT_PX) return 2
  return 1
}

function estimateMemberGridWidth(rosterWidth: number, viewportWidth: number) {
  const contentWidth = Math.max(0, rosterWidth - GROUP_CARD_HORIZONTAL_PADDING_PX)
  if (viewportWidth >= GROUP_SUMMARY_GRID_BREAKPOINT_PX) {
    return Math.max(
      0,
      contentWidth - GROUP_SUMMARY_COLUMN_WIDTH_PX - GROUP_SUMMARY_GRID_GAP_PX,
    )
  }
  return contentWidth
}

function estimateGroupCardHeight(
  group: UpstreamAccountsGroupedRosterGroup | undefined,
  memberLayout: 'list' | 'grid',
  viewportWidth: number,
) {
  if (!group) {
    return GROUP_SUMMARY_ESTIMATE_PX + GROUP_CARD_VERTICAL_GAP_PX
  }

  if (memberLayout === 'grid') {
    const columnCount = Math.max(1, resolveGridColumnCount(viewportWidth))
    const rowCount = Math.max(1, Math.ceil(group.items.length / columnCount))
    return (
      GROUP_SUMMARY_ESTIMATE_PX +
      rowCount * GROUP_MEMBER_GRID_CARD_ESTIMATE_PX +
      Math.max(0, rowCount - 1) * GROUP_MEMBER_GRID_GAP_PX +
      72
    )
  }

  return (
    GROUP_SUMMARY_ESTIMATE_PX +
    group.items.length * GROUP_MEMBER_ROW_ESTIMATE_PX +
    Math.max(0, group.items.length - 1) * GROUP_MEMBER_ROW_GAP_PX +
    56
  )
}

function buildFallbackVirtualGroups(
  groups: UpstreamAccountsGroupedRosterGroup[],
  memberLayout: 'list' | 'grid',
  viewportWidth: number,
) {
  const count = Math.min(groups.length, 4)
  let cursor = 0
  return Array.from({ length: count }, (_, index) => {
    const size =
      estimateGroupCardHeight(groups[index], memberLayout, viewportWidth) +
      (index === groups.length - 1 ? 0 : GROUP_CARD_VERTICAL_GAP_PX)
    const item = {
      key: index,
      index,
      start: cursor,
      size,
      end: cursor + size,
    }
    cursor += size
    return item
  })
}

function GroupSummaryPanel({
  group,
  groupLabels,
  compact = false,
  canEditGroupSettings = false,
  onEditGroupSettings,
}: {
  group: UpstreamAccountsGroupedRosterGroup
  groupLabels: UpstreamAccountsGroupedRosterProps['groupLabels']
  compact?: boolean
  canEditGroupSettings?: boolean
  onEditGroupSettings?: (group: UpstreamAccountsGroupedRosterGroup) => void
}) {
  const showSettingsAction =
    canEditGroupSettings &&
    Boolean(group.groupName) &&
    typeof onEditGroupSettings === 'function'

  return (
    <div
      className={cn(
        'flex flex-col gap-2 xl:pr-3.5',
        !compact && 'xl:border-r xl:border-base-300/65',
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
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
        </div>
        {showSettingsAction ? (
          <Button
            type="button"
            size="icon"
            variant={group.hasCustomSettings ? 'secondary' : 'outline'}
            className="h-9 w-9 shrink-0 rounded-full"
            aria-label={groupLabels.settingsLabel}
            title={groupLabels.settingsLabel}
            onClick={() => onEditGroupSettings?.(group)}
          >
            <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
          </Button>
        ) : null}
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

function GroupMembersList({
  items,
  selectedId,
  selectedAccountIds,
  onSelect,
  onToggleSelected,
  labels,
  memberLayout = 'list',
  selectionMode = 'multi',
  gridColumnCount,
  containerRef,
}: {
  items: UpstreamAccountSummary[]
  selectedId: number | null
  selectedAccountIds: Set<number>
  onSelect: (accountId: number) => void
  onToggleSelected?: (accountId: number, checked: boolean) => void
  labels: UpstreamAccountsTableLabels
  memberLayout?: 'list' | 'grid'
  selectionMode?: 'multi' | 'none'
  gridColumnCount: number
  containerRef?: (node: HTMLDivElement | null) => void
}) {
  if (memberLayout === 'grid') {
    return (
      <div
        ref={containerRef}
        className="self-start min-w-0 py-1"
        data-testid="upstream-accounts-group-members-grid"
      >
        <div
          className="grid gap-3"
          style={{
            gridTemplateColumns: `repeat(${Math.max(1, gridColumnCount)}, minmax(0, 1fr))`,
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
      </div>
    )
  }

  return (
    <div ref={containerRef} className="min-w-0" data-testid="upstream-accounts-group-members">
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
  canEditGroupSettings = false,
  onEditGroupSettings,
  groupLabels,
}: UpstreamAccountsGroupedRosterProps) {
  const selectAllRef = useRef<HTMLInputElement | null>(null)
  const [containerElement, setContainerElement] = useState<HTMLDivElement | null>(null)
  const [spacerElement, setSpacerElement] = useState<HTMLDivElement | null>(null)
  const [memberElement, setMemberElement] = useState<HTMLDivElement | null>(null)
  const [scrollMargin, setScrollMargin] = useState(0)
  const [rosterWidth, setRosterWidth] = useState(0)
  const [memberViewportWidth, setMemberViewportWidth] = useState(0)
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window === 'undefined' ? 0 : window.innerWidth,
  )
  const effectiveMemberViewportWidth =
    memberViewportWidth > 0
      ? memberViewportWidth
      : estimateMemberGridWidth(rosterWidth, viewportWidth)
  const gridColumnCount = Math.max(1, resolveGridColumnCount(effectiveMemberViewportWidth))
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

  const estimateSize = (index: number) =>
    estimateGroupCardHeight(groups[index], memberLayout, effectiveMemberViewportWidth) +
    (index === groups.length - 1 ? 0 : GROUP_CARD_VERTICAL_GAP_PX)

  const groupVirtualizer = useWindowVirtualizer({
    count: groups.length,
    estimateSize,
    overscan: 3,
    scrollMargin,
  })

  useEffect(() => {
    if (selectAllRef.current) {
      selectAllRef.current.indeterminate = partiallySelected
    }
  }, [partiallySelected])

  useEffect(() => {
    groupVirtualizer.measure()
  }, [groupVirtualizer, groups, memberLayout, effectiveMemberViewportWidth])

  useEffect(() => {
    const updateMetrics = () => {
      const measurementTarget = spacerElement ?? containerElement
      if (typeof window === 'undefined') {
        setRosterWidth(0)
        setMemberViewportWidth(0)
        setViewportWidth(0)
        setScrollMargin(0)
        return
      }

      setViewportWidth((current) =>
        Math.abs(current - window.innerWidth) > 0.5 ? window.innerWidth : current,
      )

      if (!measurementTarget) {
        setRosterWidth(0)
        setMemberViewportWidth(0)
        setScrollMargin(0)
        return
      }

      const nextRosterWidth = measurementTarget.getBoundingClientRect().width
      setRosterWidth((current) =>
        Math.abs(current - nextRosterWidth) > 0.5 ? nextRosterWidth : current,
      )
      const nextMemberViewportWidth =
        memberElement?.getBoundingClientRect().width ??
        estimateMemberGridWidth(nextRosterWidth, window.innerWidth)
      setMemberViewportWidth((current) =>
        Math.abs(current - nextMemberViewportWidth) > 0.5
          ? nextMemberViewportWidth
          : current,
      )
      const nextScrollMargin = measurementTarget.getBoundingClientRect().top + window.scrollY
      setScrollMargin((current) =>
        Math.abs(current - nextScrollMargin) > 0.5 ? nextScrollMargin : current,
      )
    }

    updateMetrics()
    if (!containerElement) return

    window.addEventListener('resize', updateMetrics)

    if (typeof ResizeObserver === 'undefined') {
      return () => {
        window.removeEventListener('resize', updateMetrics)
      }
    }

    const observer = new ResizeObserver(() => {
      updateMetrics()
    })
    observer.observe(containerElement)
    if (spacerElement && spacerElement !== containerElement) {
      observer.observe(spacerElement)
    }
    if (memberElement && memberElement !== spacerElement && memberElement !== containerElement) {
      observer.observe(memberElement)
    }
    if (document.body) {
      observer.observe(document.body)
    }

    return () => {
      observer.disconnect()
      window.removeEventListener('resize', updateMetrics)
    }
  }, [containerElement, spacerElement, memberElement, memberLayout, selectionEnabled])

  const virtualGroups = groupVirtualizer.getVirtualItems()
  const renderedGroups =
    virtualGroups.length > 0
      ? virtualGroups
      : buildFallbackVirtualGroups(groups, memberLayout, effectiveMemberViewportWidth)
  const normalizedVirtualGroups = renderedGroups.map((virtualGroup) => ({
    ...virtualGroup,
    start: Math.max(0, virtualGroup.start - scrollMargin),
    end: Math.max(0, virtualGroup.end - scrollMargin),
  }))
  const totalMeasuredSize =
    virtualGroups.length > 0
      ? Math.max(0, groupVirtualizer.getTotalSize())
      : groups.reduce((sum, _, index) => sum + estimateSize(index), 0)
  const paddingTop =
    normalizedVirtualGroups.length > 0 ? normalizedVirtualGroups[0]!.start : 0
  const paddingBottom =
    normalizedVirtualGroups.length > 0
      ? Math.max(
          0,
          totalMeasuredSize - normalizedVirtualGroups[normalizedVirtualGroups.length - 1]!.end,
        )
      : 0
  const firstRenderedGroupIndex =
    normalizedVirtualGroups.length > 0 ? normalizedVirtualGroups[0]!.index : null

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
      ref={setContainerElement}
      className={cn(
        'relative',
        isLoading && 'pointer-events-none select-none opacity-60',
      )}
      data-testid="upstream-accounts-grouped-roster"
      aria-busy={isLoading ? 'true' : undefined}
    >
      {selectionEnabled && onToggleSelectAllVisible ? (
        <div className="mb-4 flex items-center justify-between rounded-[0.9rem] border border-base-300/70 bg-base-100/80 px-3 py-2.5 shadow-sm backdrop-blur">
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

      <div
        ref={setSpacerElement}
        data-testid="upstream-accounts-grouped-roster-spacer"
        style={{ paddingTop: `${paddingTop}px`, paddingBottom: `${paddingBottom}px` }}
      >
        {normalizedVirtualGroups.map((virtualGroup) => {
          const group = groups[virtualGroup.index]
          if (!group) return null
          return (
            <div
              key={group.id}
              ref={groupVirtualizer.measureElement}
              data-index={virtualGroup.index}
              data-testid="upstream-accounts-group-card"
              className={cn('w-full', virtualGroup.index === groups.length - 1 ? '' : 'pb-4')}
            >
              <article className="rounded-[1.1rem] border border-base-300/65 bg-base-100/76 px-3.5 py-3 shadow-[0_8px_24px_rgba(2,6,23,0.06)]">
                <div className="grid items-start gap-3.5 xl:grid-cols-[12.5rem_minmax(0,1fr)]">
                  <GroupSummaryPanel
                    group={group}
                    groupLabels={groupLabels}
                    compact={memberLayout === 'grid'}
                    canEditGroupSettings={canEditGroupSettings}
                    onEditGroupSettings={onEditGroupSettings}
                  />

                  <GroupMembersList
                    items={group.items}
                    selectedId={selectedId}
                    selectedAccountIds={selectedAccountIds}
                    onSelect={onSelect}
                    onToggleSelected={onToggleSelected}
                    labels={labels}
                    memberLayout={memberLayout}
                    selectionMode={selectionMode}
                    gridColumnCount={gridColumnCount}
                    containerRef={
                      virtualGroup.index === firstRenderedGroupIndex ? setMemberElement : undefined
                    }
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
