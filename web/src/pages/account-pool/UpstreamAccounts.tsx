import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react'
import { AppIcon, type AppIconName } from '../../components/AppIcon'
import { AccountDetailDrawerShell } from '../../components/AccountDetailDrawerShell'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { Alert } from '../../components/ui/alert'
import { Badge } from '../../components/ui/badge'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog'
import { FloatingFieldError } from '../../components/ui/floating-field-error'
import { FormFieldFeedback } from '../../components/ui/form-field-feedback'
import { Input } from '../../components/ui/input'
import { Popover, PopoverArrow, PopoverContent, PopoverTrigger } from '../../components/ui/popover'
import { formFieldSpanVariants } from '../../components/ui/form-control'
import { SelectField } from '../../components/ui/select-field'
import { SegmentedControl, SegmentedControlItem } from '../../components/ui/segmented-control'
import { MotherAccountBadge, MotherAccountToggle } from '../../components/MotherAccountToggle'
import { Spinner } from '../../components/ui/spinner'
import { Switch } from '../../components/ui/switch'
import { AccountTagField } from '../../components/AccountTagField'
import { AccountTagFilterCombobox } from '../../components/AccountTagFilterCombobox'
import { EffectiveRoutingRuleCard } from '../../components/EffectiveRoutingRuleCard'
import { MultiSelectFilterCombobox } from '../../components/MultiSelectFilterCombobox'
import { UpstreamAccountGroupCombobox } from '../../components/UpstreamAccountGroupCombobox'
import { UpstreamAccountGroupNoteDialog } from '../../components/UpstreamAccountGroupNoteDialog'
import { UpstreamAccountUsageCard } from '../../components/UpstreamAccountUsageCard'
import { StickyKeyConversationTable } from '../../components/StickyKeyConversationTable'
import { UpstreamAccountsTable } from '../../components/UpstreamAccountsTable'
import { usePoolTags } from '../../hooks/usePoolTags'
import { useMotherSwitchNotifications } from '../../hooks/useMotherSwitchNotifications'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import { useUpstreamStickyConversations } from '../../hooks/useUpstreamStickyConversations'
import type {
  BulkUpstreamAccountActionPayload,
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncRow,
  BulkUpstreamAccountSyncSnapshot,
  PoolRoutingMaintenanceSettings,
  CompactSupportState,
  PoolRoutingTimeoutSettings,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from '../../lib/api'
import { DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS } from '../../lib/api'
import {
  createBulkUpstreamAccountSyncJobEventSource,
  normalizeBulkUpstreamAccountSyncFailedEventPayload,
  normalizeBulkUpstreamAccountSyncRowEventPayload,
  normalizeBulkUpstreamAccountSyncSnapshotEventPayload,
} from '../../lib/api'
import {
  buildGroupNameSuggestions,
  isExistingGroup,
  normalizeGroupName,
  resolveGroupNote,
} from '../../lib/upstreamAccountGroups'
import { validateUpstreamBaseUrl } from '../../lib/upstreamBaseUrl'
import { generatePoolRoutingKey } from '../../lib/poolRouting'
import { applyMotherUpdateToItems } from '../../lib/upstreamMother'
import { upstreamPlanBadgeRecipe } from '../../lib/upstreamAccountBadges'
import { cn } from '../../lib/utils'
import { useTranslation, type TranslationValues } from '../../i18n'

type AccountDraft = {
  displayName: string
  groupName: string
  isMother: boolean
  note: string
  upstreamBaseUrl: string
  tagIds: number[]
  localPrimaryLimit: string
  localSecondaryLimit: string
  localLimitUnit: string
  apiKey: string
}

type RoutingDraft = {
  apiKey: string
  maskedApiKey: string | null
  primarySyncIntervalSecs: string
  secondarySyncIntervalSecs: string
  priorityAvailableAccountCap: string
  responsesFirstByteTimeoutSecs: string
  compactFirstByteTimeoutSecs: string
  responsesStreamTimeoutSecs: string
  compactStreamTimeoutSecs: string
}

const DEFAULT_ROUTING_TIMEOUTS: PoolRoutingTimeoutSettings = {
  responsesFirstByteTimeoutSecs: 120,
  compactFirstByteTimeoutSecs: 300,
  responsesStreamTimeoutSecs: 300,
  compactStreamTimeoutSecs: 300,
}
const POSITIVE_INTEGER_PATTERN = /^[1-9]\d*$/

const STICKY_CONVERSATION_LIMIT_OPTIONS = [20, 50, 100] as const

type UpstreamAccountsLocationState = {
  selectedAccountId?: number
  openDetail?: boolean
  openDeleteConfirm?: boolean
  postCreateWarning?: string | null
  duplicateWarning?: {
    accountId: number
    displayName: string
    peerAccountIds: number[]
    reasons: string[]
  } | null
}

type GroupSettingsEditorState = {
  open: boolean
  groupName: string
  note: string
  existing: boolean
  boundProxyKeys: string[]
}

type OauthRecoveryHint = {
  titleKey: string
  bodyKey: string
}

type ActionErrorState = {
  routing: string | null
  accountMessages: Record<number, string>
}

type AccountBusyActionType = 'save' | 'sync' | 'toggle' | 'relogin' | 'delete'

type BusyActionState = {
  routing: boolean
  accountActions: Set<string>
}

type AccountDetailTab = 'overview' | 'edit' | 'routing' | 'healthEvents'

function createBusyActionKey(type: AccountBusyActionType, accountId: number) {
  return `${type}:${accountId}`
}

function isBusyAction(
  busyAction: BusyActionState,
  type: AccountBusyActionType | 'routing',
  accountId?: number,
) {
  if (type === 'routing') return busyAction.routing
  if (typeof accountId !== 'number') return false
  return busyAction.accountActions.has(createBusyActionKey(type, accountId))
}

function hasBusyAccountAction(busyAction: BusyActionState, accountId?: number | null) {
  if (typeof accountId !== 'number') return false
  const suffix = `:${accountId}`
  for (const key of busyAction.accountActions) {
    if (key.endsWith(suffix)) return true
  }
  return false
}

function formatDateTime(value?: string | null) {
  if (!value) return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(date)
}


function normalizeNumberInput(value: string): number | undefined {
  const trimmed = value.trim()
  if (!trimmed) return undefined
  const parsed = Number(trimmed)
  return Number.isFinite(parsed) ? parsed : undefined
}

function normalizeDisplayNameKey(value: string) {
  return value.trim().toLocaleLowerCase()
}

function findDisplayNameConflict(
  items: UpstreamAccountSummary[],
  displayName: string,
  excludeId?: number | null,
) {
  const normalized = normalizeDisplayNameKey(displayName)
  if (!normalized) return null
  return (
    items.find(
      (item) =>
        item.id !== excludeId &&
        normalizeDisplayNameKey(item.displayName) === normalized,
    ) ?? null
  )
}

function buildDraft(detail: UpstreamAccountDetail | null): AccountDraft {
  return {
    displayName: detail?.displayName ?? '',
    groupName: detail?.groupName ?? '',
    isMother: detail?.isMother ?? false,
    note: detail?.note ?? '',
    upstreamBaseUrl: detail?.upstreamBaseUrl ?? '',
    tagIds: detail?.tags?.map((tag) => tag.id) ?? [],
    localPrimaryLimit:
      detail?.localLimits?.primaryLimit == null ? '' : String(detail.localLimits.primaryLimit),
    localSecondaryLimit:
      detail?.localLimits?.secondaryLimit == null ? '' : String(detail.localLimits.secondaryLimit),
    localLimitUnit: detail?.localLimits?.limitUnit ?? 'requests',
    apiKey: '',
  }
}

function resolveRoutingMaintenance(
  maintenance?: PoolRoutingMaintenanceSettings | null,
): PoolRoutingMaintenanceSettings {
  return {
    primarySyncIntervalSecs:
      maintenance?.primarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
    secondarySyncIntervalSecs:
      maintenance?.secondarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
    priorityAvailableAccountCap:
      maintenance?.priorityAvailableAccountCap ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
  }
}

function buildRoutingDraft(
  routing?: {
    maskedApiKey?: string | null
    maintenance?: PoolRoutingMaintenanceSettings | null
    timeouts?: PoolRoutingTimeoutSettings | null
  } | null,
): RoutingDraft {
  const maintenance = resolveRoutingMaintenance(routing?.maintenance)
  const timeouts = routing?.timeouts ?? DEFAULT_ROUTING_TIMEOUTS
  return {
    apiKey: '',
    maskedApiKey: routing?.maskedApiKey ?? null,
    primarySyncIntervalSecs: String(maintenance.primarySyncIntervalSecs),
    secondarySyncIntervalSecs: String(maintenance.secondarySyncIntervalSecs),
    priorityAvailableAccountCap: String(maintenance.priorityAvailableAccountCap),
    responsesFirstByteTimeoutSecs: String(timeouts.responsesFirstByteTimeoutSecs),
    compactFirstByteTimeoutSecs: String(timeouts.compactFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: String(timeouts.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: String(timeouts.compactStreamTimeoutSecs),
  }
}

type AccountStatusSnapshot = Pick<
  UpstreamAccountSummary,
  'status' | 'displayStatus' | 'enabled' | 'workStatus' | 'enableStatus' | 'healthStatus' | 'syncState'
>

function accountEnableStatus(item?: AccountStatusSnapshot | null) {
  if (item?.enableStatus) return item.enableStatus
  if (item?.enabled === false || item?.displayStatus === 'disabled') return 'disabled'
  return 'enabled'
}

function accountWorkStatus(item?: AccountStatusSnapshot | null) {
  if (!item) return 'idle'
  if (accountEnableStatus(item) !== 'enabled') return 'idle'
  if (accountSyncState(item) === 'syncing') return 'idle'
  if (item?.workStatus === 'rate_limited') return 'rate_limited'
  if (accountHealthStatus(item) !== 'normal') return 'unavailable'
  return item?.workStatus ?? 'idle'
}

function accountHealthStatus(item?: AccountStatusSnapshot | null) {
  if (item?.healthStatus) return item.healthStatus
  const legacyStatus = item?.displayStatus ?? item?.status ?? 'error_other'
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

function accountSyncState(item?: AccountStatusSnapshot | null) {
  if (item?.syncState) return item.syncState
  const legacyStatus = item?.displayStatus ?? item?.status
  return legacyStatus === 'syncing' ? 'syncing' : 'idle'
}

function parseRoutingPositiveInteger(value: string) {
  const trimmed = value.trim()
  if (!trimmed || !/^\d+$/.test(trimmed)) return null
  const parsed = Number(trimmed)
  return Number.isSafeInteger(parsed) ? parsed : null
}

function enableStatusVariant(status: string): 'success' | 'secondary' {
  return status === 'enabled' ? 'success' : 'secondary'
}

function workStatusVariant(status: string): 'info' | 'warning' | 'secondary' {
  if (status === 'working') return 'info'
  if (status === 'rate_limited') return 'warning'
  return 'secondary'
}

function healthStatusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'normal') return 'success'
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

function syncStateVariant(status: string): 'warning' | 'secondary' {
  return status === 'syncing' ? 'warning' : 'secondary'
}

function bulkSyncRowStatusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'succeeded') return 'success'
  if (status === 'pending') return 'warning'
  if (status === 'failed') return 'error'
  return 'secondary'
}

function computeBulkSyncCounts(rows: BulkUpstreamAccountSyncRow[]): BulkUpstreamAccountSyncCounts {
  return rows.reduce<BulkUpstreamAccountSyncCounts>((counts, row) => {
    counts.total += 1
    if (row.status === 'succeeded') {
      counts.succeeded += 1
      counts.completed += 1
    } else if (row.status === 'failed') {
      counts.failed += 1
      counts.completed += 1
    } else if (row.status === 'skipped') {
      counts.skipped += 1
      counts.completed += 1
    }
    return counts
  }, {
    total: 0,
    completed: 0,
    succeeded: 0,
    failed: 0,
    skipped: 0,
  })
}

function resolveBulkSyncCounts(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts?: BulkUpstreamAccountSyncCounts | null,
) {
  return counts ?? computeBulkSyncCounts(snapshot.rows)
}

function withBulkSyncSnapshotStatus(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  status: BulkUpstreamAccountSyncSnapshot['status'],
) {
  if (snapshot.status === status) return snapshot
  return {
    ...snapshot,
    status,
  }
}

function shouldAutoHideBulkSyncProgress(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts: BulkUpstreamAccountSyncCounts,
) {
  return snapshot.status === 'completed' && counts.failed === 0 && counts.skipped === 0
}

function kindVariant(kind: string): 'secondary' | 'success' {
  return kind === 'oauth_codex' ? 'success' : 'secondary'
}

function isLegacyOauthBridgeExchangeError(lastError?: string | null) {
  const normalized = lastError?.toLocaleLowerCase() ?? ''
  return normalized.includes('oauth bridge token exchange failed')
}

function resolveOauthRecoveryHint(
  kind: string,
  healthStatus: string,
  lastError?: string | null,
): OauthRecoveryHint | null {
  if (kind !== 'oauth_codex') return null
  if (isLegacyOauthBridgeExchangeError(lastError)) {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.bridgeExchangeTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.bridgeExchangeBody',
    }
  }
  if (healthStatus === 'upstream_unavailable') {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.dataPlaneUnavailableTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.dataPlaneUnavailableBody',
    }
  }
  if (healthStatus === 'upstream_rejected') {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.dataPlaneRejectedTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.dataPlaneRejectedBody',
    }
  }
  if (healthStatus === 'needs_reauth') {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.reauthTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.reauthBody',
    }
  }
  return null
}

function compactSupportLabel(
  support: CompactSupportState | null | undefined,
  t: (key: string) => string,
) {
  if (!support || support.status !== 'unsupported') return null
  return t('accountPool.upstreamAccounts.compactSupport.unsupportedBadge')
}

function compactSupportHint(
  support: CompactSupportState | null | undefined,
  t: (key: string, values?: TranslationValues) => string,
) {
  if (!support || support.status === 'unknown') return null
  const statusLabel =
    support.status === 'unsupported'
      ? t('accountPool.upstreamAccounts.compactSupport.status.unsupported')
      : t('accountPool.upstreamAccounts.compactSupport.status.supported')
  const observedAt = support.observedAt
    ? formatDateTime(support.observedAt)
    : t('accountPool.upstreamAccounts.unavailable')
  if (support.reason) {
    return `${statusLabel} · ${observedAt} · ${support.reason}`
  }
  return `${statusLabel} · ${observedAt}`
}

function parseRoutingTimeoutValue(
  raw: string,
  label: string,
): { ok: true; value: number } | { ok: false; error: string } {
  const trimmed = raw.trim()
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` }
  }
  if (!POSITIVE_INTEGER_PATTERN.test(trimmed)) {
    return { ok: false, error: `${label} must be a positive integer.` }
  }
  const parsed = Number(trimmed)
  if (!Number.isSafeInteger(parsed)) {
    return { ok: false, error: `${label} must be a positive integer.` }
  }
  return { ok: true, value: parsed }
}


function poolCardMetric(value: number, label: string, icon: AppIconName, accent: string) {
  return { value, label, icon, accent }
}

function AccountDetailSkeleton() {
  return (
    <div className="grid gap-4">
      {Array.from({ length: 3 }).map((_, index) => (
        <div key={index} className="h-28 animate-pulse rounded-[1.35rem] bg-base-200/75" />
      ))}
    </div>
  )
}

function DetailField({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric-cell">
      <p className="metric-label">{label}</p>
      <p className="mt-2 break-all text-sm text-base-content/80">{value || '—'}</p>
    </div>
  )
}

function RoutingSettingsDialog({
  open,
  title,
  description,
  closeLabel,
  cancelLabel,
  saveLabel,
  apiKey,
  primarySyncIntervalSecs,
  secondarySyncIntervalSecs,
  priorityAvailableAccountCap,
  timeoutSectionTitle,
  timeoutFields,
  busy,
  apiKeyWritesEnabled,
  timeoutWritesEnabled,
  canSave,
  onApiKeyChange,
  onGenerate,
  onPrimarySyncIntervalChange,
  onSecondarySyncIntervalChange,
  onPriorityAvailableAccountCapChange,
  onClose,
  onSave,
}: {
  open: boolean
  title: string
  description: string
  closeLabel: string
  cancelLabel: string
  saveLabel: string
  apiKey: string
  primarySyncIntervalSecs: string
  secondarySyncIntervalSecs: string
  priorityAvailableAccountCap: string
  timeoutSectionTitle: string
  timeoutFields: Array<{
    key: string
    label: string
    value: string
    onChange: (value: string) => void
  }>
  busy: boolean
  apiKeyWritesEnabled: boolean
  timeoutWritesEnabled: boolean
  canSave: boolean
  onApiKeyChange: (value: string) => void
  onGenerate: () => void
  onPrimarySyncIntervalChange: (value: string) => void
  onSecondarySyncIntervalChange: (value: string) => void
  onPriorityAvailableAccountCapChange: (value: string) => void
  onClose: () => void
  onSave: () => void
}) {
  const { t } = useTranslation()
  const apiKeyInputRef = useRef<HTMLInputElement | null>(null)
  const primaryInputRef = useRef<HTMLInputElement | null>(null)
  const apiKeyInputId = 'pool-routing-secret-input'
  const primaryInputId = 'pool-routing-primary-sync-interval'
  const secondaryInputId = 'pool-routing-secondary-sync-interval'
  const capInputId = 'pool-routing-priority-cap'

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}>
      <DialogContent
        className="flex max-h-[calc(100dvh-2rem)] flex-col overflow-hidden p-0 sm:max-h-[calc(100dvh-4rem)]"
        onOpenAutoFocus={(event) => {
          event.preventDefault()
          if (apiKeyWritesEnabled) {
            apiKeyInputRef.current?.focus()
            return
          }
          primaryInputRef.current?.focus()
        }}
        onPointerDownOutside={(event) => {
          if (busy) event.preventDefault()
        }}
        onEscapeKeyDown={(event) => {
          if (busy) event.preventDefault()
        }}
      >
        <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
          <DialogHeader className="min-w-0 max-w-[28rem]">
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
          <div className="space-y-4">
            <div className="space-y-3 rounded-2xl border border-base-300/80 bg-base-100/70 p-4">
              <div className="space-y-1">
                <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t('accountPool.upstreamAccounts.routing.apiKeySectionTitle')}
                </p>
                <p className="text-sm text-base-content/68">
                  {t('accountPool.upstreamAccounts.routing.apiKeySectionDescription')}
                </p>
              </div>
              <div className="field">
                <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
                  <label htmlFor={apiKeyInputId} className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                    {t('accountPool.upstreamAccounts.routing.apiKeyLabel')}
                  </label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={onGenerate}
                    disabled={busy || !apiKeyWritesEnabled}
                  >
                    <AppIcon name="auto-fix" className="mr-2 h-4 w-4" aria-hidden />
                    {t('accountPool.upstreamAccounts.routing.generate')}
                  </Button>
                </div>
                <Input
                  id={apiKeyInputId}
                  ref={apiKeyInputRef}
                  name="poolRoutingSecret"
                  type="text"
                  value={apiKey}
                  onChange={(event) => onApiKeyChange(event.target.value)}
                  placeholder={t('accountPool.upstreamAccounts.routing.apiKeyPlaceholder')}
                  autoComplete="off"
                  autoCorrect="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  data-1p-ignore="true"
                  data-lpignore="true"
                  disabled={busy || !apiKeyWritesEnabled}
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono placeholder:text-base-content/58"
                />
              </div>
            </div>

            <div className="space-y-4 rounded-2xl border border-base-300/80 bg-base-100/70 p-4">
              <div className="space-y-1">
                <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t('accountPool.upstreamAccounts.routing.maintenanceSectionTitle')}
                </p>
                <p className="text-sm text-base-content/68">
                  {t('accountPool.upstreamAccounts.routing.maintenanceSectionDescription')}
                </p>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="field">
                  <label htmlFor={primaryInputId} className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                    {t('accountPool.upstreamAccounts.routing.primarySyncIntervalLabel')}
                  </label>
                  <Input
                    id={primaryInputId}
                    ref={primaryInputRef}
                    name="primarySyncIntervalSecs"
                    type="number"
                    min={60}
                    step={60}
                    inputMode="numeric"
                    value={primarySyncIntervalSecs}
                    onChange={(event) => onPrimarySyncIntervalChange(event.target.value)}
                    placeholder="300"
                    disabled={busy || !timeoutWritesEnabled}
                    className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                  />
                </div>
                <div className="field">
                  <label htmlFor={secondaryInputId} className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                    {t('accountPool.upstreamAccounts.routing.secondarySyncIntervalLabel')}
                  </label>
                  <Input
                    id={secondaryInputId}
                    name="secondarySyncIntervalSecs"
                    type="number"
                    min={60}
                    step={60}
                    inputMode="numeric"
                    value={secondarySyncIntervalSecs}
                    onChange={(event) => onSecondarySyncIntervalChange(event.target.value)}
                    placeholder="1800"
                    disabled={busy || !timeoutWritesEnabled}
                    className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                  />
                </div>
              </div>
              <div className="field">
                <label htmlFor={capInputId} className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t('accountPool.upstreamAccounts.routing.priorityCapLabel')}
                </label>
                <Input
                  id={capInputId}
                  name="priorityAvailableAccountCap"
                  type="number"
                  min={1}
                  step={1}
                  inputMode="numeric"
                  value={priorityAvailableAccountCap}
                  onChange={(event) => onPriorityAvailableAccountCapChange(event.target.value)}
                  placeholder="100"
                  disabled={busy || !timeoutWritesEnabled}
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                />
              </div>
            </div>
            <div className="space-y-3">
              <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                {timeoutSectionTitle}
              </p>
              <div className="grid gap-3 md:grid-cols-2">
                {timeoutFields.map((field) => (
                  <label key={field.key} className="field">
                    <span className="field-label">{field.label}</span>
                    <Input
                      name={field.key}
                      type="number"
                      min="1"
                      step="1"
                      value={field.value}
                      onChange={(event) => field.onChange(event.target.value)}
                      disabled={busy || !timeoutWritesEnabled}
                      className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                    />
                  </label>
                ))}
              </div>
            </div>
          </div>
        </div>
        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy || !canSave}>
            {busy ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="key-chain-variant" className="mr-2 h-4 w-4" aria-hidden />}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export default function UpstreamAccountsPage() {
  const { t, locale } = useTranslation()
  const location = useLocation()
  const navigate = useNavigate()
  const [groupFilterQuery, setGroupFilterQuery] = useState('')
  const [selectedTagIds, setSelectedTagIds] = useState<number[]>([])
  const [workStatusFilter, setWorkStatusFilter] = useState<string[]>([])
  const [enableStatusFilter, setEnableStatusFilter] = useState<string[]>([])
  const [healthStatusFilter, setHealthStatusFilter] = useState<string[]>([])
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)
  const [selectedAccountIds, setSelectedAccountIds] = useState<number[]>([])
  const [selectedAccountSummaries, setSelectedAccountSummaries] = useState<Record<number, UpstreamAccountSummary>>({})
  const accountListQuery = useMemo(() => {
    const normalizedQuery = groupFilterQuery.trim()
    const allLabel = t('accountPool.upstreamAccounts.groupFilter.all').toLocaleLowerCase()
    const ungroupedLabel = t('accountPool.upstreamAccounts.groupFilter.ungrouped').toLocaleLowerCase()
    const normalizedLowerQuery = normalizedQuery.toLocaleLowerCase()

    return {
      groupSearch:
        !normalizedQuery || normalizedLowerQuery === allLabel || normalizedLowerQuery === ungroupedLabel
          ? undefined
          : normalizedQuery,
      groupUngrouped: normalizedQuery ? normalizedLowerQuery === ungroupedLabel : undefined,
      workStatus: workStatusFilter.length > 0 ? workStatusFilter : undefined,
      enableStatus: enableStatusFilter.length > 0 ? enableStatusFilter : undefined,
      healthStatus: healthStatusFilter.length > 0 ? healthStatusFilter : undefined,
      page,
      pageSize,
      tagIds: selectedTagIds.length > 0 ? selectedTagIds : undefined,
    }
  }, [enableStatusFilter, groupFilterQuery, healthStatusFilter, page, pageSize, selectedTagIds, t, workStatusFilter])
  const workStatusFilterOptions = useMemo(
    () => [
      { value: 'working', label: t('accountPool.upstreamAccounts.workStatus.working') },
      { value: 'idle', label: t('accountPool.upstreamAccounts.workStatus.idle') },
      { value: 'rate_limited', label: t('accountPool.upstreamAccounts.workStatus.rate_limited') },
      { value: 'unavailable', label: t('accountPool.upstreamAccounts.workStatus.unavailable') },
    ],
    [t],
  )
  const enableStatusFilterOptions = useMemo(
    () => [
      { value: 'enabled', label: t('accountPool.upstreamAccounts.enableStatus.enabled') },
      { value: 'disabled', label: t('accountPool.upstreamAccounts.enableStatus.disabled') },
    ],
    [t],
  )
  const healthStatusFilterOptions = useMemo(
    () => [
      { value: 'normal', label: t('accountPool.upstreamAccounts.healthStatus.normal') },
      { value: 'needs_reauth', label: t('accountPool.upstreamAccounts.healthStatus.needs_reauth') },
      {
        value: 'upstream_unavailable',
        label: t('accountPool.upstreamAccounts.healthStatus.upstream_unavailable'),
      },
      {
        value: 'upstream_rejected',
        label: t('accountPool.upstreamAccounts.healthStatus.upstream_rejected'),
      },
      { value: 'error_other', label: t('accountPool.upstreamAccounts.healthStatus.error_other') },
    ],
    [t],
  )
  const pageSizeOptions = useMemo(
    () => [20, 50, 100].map((value) => ({ value: String(value), label: String(value) })),
    [],
  )
  const {
    items,
    groups = [],
    forwardProxyNodes = [],
    hasUngroupedAccounts = false,
    writesEnabled,
    selectedId,
    selectedSummary,
    detail,
    isLoading,
    isDetailLoading,
    listError = null,
    detailError = null,
    selectAccount,
    refresh,
    saveAccount,
    runSync,
    removeAccount,
    routing,
    saveRouting,
    saveGroupNote,
    runBulkAction,
    startBulkSyncJob,
    getBulkSyncJob,
    stopBulkSyncJob,
    total,
    metrics: listMetrics,
  } = useUpstreamAccounts(accountListQuery)
  const { items: tagItems, createTag, updateTag, deleteTag } = usePoolTags()
  const notifyMotherSwitches = useMotherSwitchNotifications()

  const [draft, setDraft] = useState<AccountDraft>(buildDraft(null))
  const [routingDraft, setRoutingDraft] = useState(() => buildRoutingDraft(null))
  const [actionError, setActionError] = useState<ActionErrorState>(() => ({
    routing: null,
    accountMessages: {},
  }))
  const [busyAction, setBusyAction] = useState<BusyActionState>(() => ({
    routing: false,
    accountActions: new Set(),
  }))
  const [isDetailDrawerOpen, setIsDetailDrawerOpen] = useState(false)
  const [isRoutingDialogOpen, setIsRoutingDialogOpen] = useState(false)
  const [isRoutingDialogInspectOnly, setIsRoutingDialogInspectOnly] = useState(false)
  const [isDeleteConfirmOpen, setIsDeleteConfirmOpen] = useState(false)
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([])
  const [stickyConversationLimit, setStickyConversationLimit] = useState<number>(50)
  const [groupDraftNotes, setGroupDraftNotes] = useState<Record<string, string>>({})
  const [groupDraftBoundProxyKeys, setGroupDraftBoundProxyKeys] = useState<Record<string, string[]>>({})
  const [postCreateWarning, setPostCreateWarning] = useState<string | null>(null)
  const [duplicateWarning, setDuplicateWarning] =
    useState<UpstreamAccountsLocationState['duplicateWarning']>(null)
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupSettingsEditorState>({
    open: false,
    groupName: '',
    note: '',
    existing: false,
    boundProxyKeys: [],
  })
  const [groupNoteBusy, setGroupNoteBusy] = useState(false)
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null)
  const [bulkActionBusy, setBulkActionBusy] = useState<string | null>(null)
  const [bulkActionMessage, setBulkActionMessage] = useState<string | null>(null)
  const [bulkActionError, setBulkActionError] = useState<string | null>(null)
  const [bulkGroupDialogOpen, setBulkGroupDialogOpen] = useState(false)
  const [bulkGroupName, setBulkGroupName] = useState('')
  const [bulkTagsDialogOpen, setBulkTagsDialogOpen] = useState(false)
  const [bulkTagMode, setBulkTagMode] = useState<'add_tags' | 'remove_tags'>('add_tags')
  const [bulkTagIds, setBulkTagIds] = useState<number[]>([])
  const [bulkDeleteDialogOpen, setBulkDeleteDialogOpen] = useState(false)
  const [bulkSyncSnapshot, setBulkSyncSnapshot] = useState<BulkUpstreamAccountSyncSnapshot | null>(null)
  const [bulkSyncCounts, setBulkSyncCounts] = useState<BulkUpstreamAccountSyncCounts | null>(null)
  const [bulkSyncError, setBulkSyncError] = useState<string | null>(null)
  const [isBulkSyncStarting, setIsBulkSyncStarting] = useState(false)
  const bulkSyncEventSourceRef = useRef<EventSource | null>(null)
  const deleteConfirmCancelRef = useRef<HTMLButtonElement | null>(null)
  const [detailDrawerPortalContainer, setDetailDrawerPortalContainer] = useState<HTMLElement | null>(null)
  const [detailTab, setDetailTab] = useState<AccountDetailTab>('overview')
  const skipNextDeleteConfirmResetRef = useRef(false)
  const deleteConfirmTitleId = useId()
  const detailDrawerTitleId = 'upstream-account-detail-title'
  const detailDrawerTabsBaseId = useId()
  const selectedIdRef = useRef<number | null>(selectedId)
  const selectedAccountIdSet = useMemo(() => new Set(selectedAccountIds), [selectedAccountIds])
  const routingWritesEnabled = routing
    ? (routing.writesEnabled ?? writesEnabled)
    : false
  const effectiveMetrics = listMetrics ?? {
    total: items.length,
    oauth: items.filter((item) => item.kind === 'oauth_codex').length,
    apiKey: items.filter((item) => item.kind === 'api_key_codex').length,
    attention: items.filter((item) =>
      accountHealthStatus(item) !== 'normal' || accountWorkStatus(item) === 'rate_limited',
    ).length,
  }
  const effectiveTotal = total ?? effectiveMetrics.total
  const pageCount = Math.max(1, Math.ceil(effectiveTotal / Math.max(pageSize, 1)))

  useEffect(() => {
    selectedIdRef.current = selectedId
  }, [selectedId])

  const clearBulkSelection = useCallback(() => {
    setSelectedAccountIds([])
    setBulkGroupDialogOpen(false)
    setBulkTagsDialogOpen(false)
    setBulkDeleteDialogOpen(false)
  }, [])

  const handleGroupFilterChange = useCallback((value: string) => {
    setGroupFilterQuery(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handleTagFilterChange = useCallback((value: number[]) => {
    setSelectedTagIds(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handleWorkStatusFilterChange = useCallback((value: string[]) => {
    setWorkStatusFilter(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handleEnableStatusFilterChange = useCallback((value: string[]) => {
    setEnableStatusFilter(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handleHealthStatusFilterChange = useCallback((value: string[]) => {
    setHealthStatusFilter(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handlePageSizeChange = useCallback((value: number) => {
    setPageSize(value)
    setPage(1)
    clearBulkSelection()
  }, [clearBulkSelection])

  const handleOpenRoutingDialog = useCallback(() => {
    setRoutingDraft(buildRoutingDraft(routing))
    setIsRoutingDialogInspectOnly(!routingWritesEnabled)
    setIsRoutingDialogOpen(true)
  }, [routing, routingWritesEnabled])

  const closeBulkSyncEventSource = useCallback(() => {
    bulkSyncEventSourceRef.current?.close()
    bulkSyncEventSourceRef.current = null
  }, [])

  const clearBulkSyncProgress = useCallback(() => {
    setBulkSyncSnapshot(null)
    setBulkSyncCounts(null)
    setBulkSyncError(null)
  }, [])

  const draftUpstreamBaseUrlError = useMemo(() => {
    const code = validateUpstreamBaseUrl(draft.upstreamBaseUrl)
    if (code === 'invalid_absolute_url') {
      return t('accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid')
    }
    if (code === 'query_or_fragment_not_allowed') {
      return t('accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment')
    }
    return null
  }, [draft.upstreamBaseUrl, t])

  useEffect(() => {
    setDraft(buildDraft(detail))
  }, [detail])

  useEffect(() => {
    if (!selectedSummary && !detail) {
      setIsDetailDrawerOpen(false)
    }
  }, [detail, selectedSummary])

  useEffect(() => {
    if (!isDetailDrawerOpen) {
      setDetailTab('overview')
    }
  }, [isDetailDrawerOpen])

  useEffect(() => {
    setDetailTab('overview')
  }, [selectedId])

  useEffect(() => {
    if (skipNextDeleteConfirmResetRef.current) {
      skipNextDeleteConfirmResetRef.current = false
      return
    }
    setIsDeleteConfirmOpen(false)
  }, [selectedId, isDetailDrawerOpen])

  useEffect(() => {
    if (isRoutingDialogOpen && !routing) {
      setRoutingDraft(buildRoutingDraft(null))
      setIsRoutingDialogInspectOnly(false)
      setIsRoutingDialogOpen(false)
      return
    }
    if (isRoutingDialogOpen) {
      if (!routingWritesEnabled) {
        setRoutingDraft(buildRoutingDraft(routing))
        setIsRoutingDialogInspectOnly(true)
        return
      }
      if (isRoutingDialogInspectOnly) {
        setRoutingDraft(buildRoutingDraft(routing))
        setIsRoutingDialogInspectOnly(false)
      }
      return
    }
    setRoutingDraft(buildRoutingDraft(routing))
  }, [
    isRoutingDialogOpen,
    isRoutingDialogInspectOnly,
    routingWritesEnabled,
    routing,
    routing?.maskedApiKey,
    routing?.writesEnabled,
    routing?.maintenance?.primarySyncIntervalSecs,
    routing?.maintenance?.secondarySyncIntervalSecs,
    routing?.maintenance?.priorityAvailableAccountCap,
    routing?.timeouts?.responsesFirstByteTimeoutSecs,
    routing?.timeouts?.compactFirstByteTimeoutSecs,
    routing?.timeouts?.responsesStreamTimeoutSecs,
    routing?.timeouts?.compactStreamTimeoutSecs,
  ])

  useEffect(() => {
    if (!writesEnabled) {
      setIsDeleteConfirmOpen(false)
    }
  }, [writesEnabled])

  useEffect(() => {
    setSelectedAccountSummaries((current) => {
      const currentPageMap = new Map(items.map((item) => [item.id, item]))
      const next: Record<number, UpstreamAccountSummary> = {}
      for (const accountId of selectedAccountIds) {
        const summary = currentPageMap.get(accountId) ?? current[accountId]
        if (summary) {
          next[accountId] = summary
        }
      }
      const currentKeys = Object.keys(current)
      const nextKeys = Object.keys(next)
      if (
        currentKeys.length === nextKeys.length &&
        nextKeys.every((key) => current[Number(key)] === next[Number(key)])
      ) {
        return current
      }
      return next
    })
  }, [items, selectedAccountIds])

  useEffect(() => {
    setGroupDraftNotes((current) => {
      const nextEntries = Object.entries(current).filter(([groupName]) => !isExistingGroup(groups, groupName))
      if (nextEntries.length === Object.keys(current).length) {
        return current
      }
      return Object.fromEntries(nextEntries)
    })
    setGroupDraftBoundProxyKeys((current) => {
      const nextEntries = Object.entries(current).filter(([groupName]) => !isExistingGroup(groups, groupName))
      if (nextEntries.length === Object.keys(current).length) {
        return current
      }
      return Object.fromEntries(nextEntries)
    })
  }, [groups])

  useEffect(() => {
    const validTagIds = new Set(tagItems.map((tag) => tag.id))
    setSelectedTagIds((current) => {
      const next = current.filter((tagId) => validTagIds.has(tagId))
      return next.length === current.length ? current : next
    })
  }, [tagItems])

  useEffect(() => {
    return () => {
      closeBulkSyncEventSource()
    }
  }, [closeBulkSyncEventSource])

  useEffect(() => {
    if (effectiveTotal > 0 && page > pageCount) {
      setPage(pageCount)
    }
  }, [effectiveTotal, page, pageCount])

  useEffect(() => {
    const state = location.state as UpstreamAccountsLocationState | null
    if (!state?.selectedAccountId) return

    skipNextDeleteConfirmResetRef.current = Boolean(state.openDeleteConfirm)
    selectAccount(state.selectedAccountId)
    setIsDetailDrawerOpen(Boolean(state.openDetail))
    setIsDeleteConfirmOpen(Boolean(state.openDeleteConfirm))
    setPostCreateWarning(state.postCreateWarning ?? null)
    setDuplicateWarning(state.duplicateWarning ?? null)
    navigate(location.pathname, { replace: true, state: null })
  }, [location.pathname, location.state, navigate, selectAccount])

  useEffect(() => {
    if (!duplicateWarning) return
    if (duplicateWarning.accountId === selectedId) return
    setDuplicateWarning(null)
  }, [duplicateWarning, selectedId])

  const handleCreateTag = async (payload: Parameters<typeof createTag>[0]) => {
    const detail = await createTag(payload)
    setPageCreatedTagIds((current) => (current.includes(detail.id) ? current : [...current, detail.id]))
    return detail
  }

  const handleDeleteTag = async (tagId: number) => {
    await deleteTag(tagId)
    setPageCreatedTagIds((current) => current.filter((value) => value !== tagId))
    setDraft((current) => ({ ...current, tagIds: current.tagIds.filter((value) => value !== tagId) }))
  }

  const metrics = useMemo(() => {
    return [
      poolCardMetric(effectiveMetrics.total, t('accountPool.upstreamAccounts.metrics.total'), 'database-outline', 'text-primary'),
      poolCardMetric(effectiveMetrics.oauth, t('accountPool.upstreamAccounts.metrics.oauth'), 'badge-account-horizontal-outline', 'text-success'),
      poolCardMetric(effectiveMetrics.apiKey, t('accountPool.upstreamAccounts.metrics.apiKey'), 'key-outline', 'text-info'),
      poolCardMetric(
        effectiveMetrics.attention,
        t('accountPool.upstreamAccounts.metrics.attention'),
        'alert-decagram-outline',
        'text-warning',
      ),
    ]
  }, [effectiveMetrics, t])

  const availableGroups = useMemo(() => {
    const draftNames = Object.fromEntries([
      ...Object.keys(groupDraftNotes).map((groupName) => [groupName, '']),
      ...Object.keys(groupDraftBoundProxyKeys).map((groupName) => [groupName, '']),
    ])
    return {
      names: buildGroupNameSuggestions(items.map((item) => item.groupName), groups, draftNames),
      hasUngrouped: hasUngroupedAccounts,
    }
  }, [groupDraftBoundProxyKeys, groupDraftNotes, groups, hasUngroupedAccounts, items])

  const resolveGroupSummaryForName = (groupName: string) => {
    const normalized = normalizeGroupName(groupName)
    if (!normalized) return null
    return groups.find((group) => normalizeGroupName(group.groupName) === normalized) ?? null
  }
  const resolveGroupNoteForName = (groupName: string) => resolveGroupNote(groups, groupDraftNotes, groupName)
  const resolvePendingGroupNoteForName = (groupName: string) => {
    const normalized = normalizeGroupName(groupName)
    if (!normalized || isExistingGroup(groups, normalized)) return ''
    return groupDraftNotes[normalized]?.trim() ?? ''
  }
  const resolveGroupBoundProxyKeysForName = (groupName: string) =>
    resolveGroupSummaryForName(groupName)?.boundProxyKeys ??
    groupDraftBoundProxyKeys[normalizeGroupName(groupName)] ??
    []
  const hasGroupSettings = (groupName: string) =>
    resolveGroupNoteForName(groupName).trim().length > 0 ||
    resolveGroupBoundProxyKeysForName(groupName).length > 0

  const persistDraftGroupSettings = useCallback(async (groupName: string) => {
    const normalizedGroupName = normalizeGroupName(groupName)
    if (!normalizedGroupName) return
    const hasDraftNote = normalizedGroupName in groupDraftNotes
    const hasDraftBindings = normalizedGroupName in groupDraftBoundProxyKeys
    if (!hasDraftNote && !hasDraftBindings) return

    const normalizedNote = hasDraftNote
      ? groupDraftNotes[normalizedGroupName]?.trim() ?? ''
      : ''
    const normalizedBoundProxyKeys = Array.from(
      new Set(
        (groupDraftBoundProxyKeys[normalizedGroupName] ?? [])
          .map((value) => value.trim())
          .filter((value) => value.length > 0),
      ),
    )

    await saveGroupNote(normalizedGroupName, {
      note: normalizedNote || undefined,
      boundProxyKeys: normalizedBoundProxyKeys,
    })

    setGroupDraftNotes((current) => {
      if (!(normalizedGroupName in current)) return current
      const next = { ...current }
      delete next[normalizedGroupName]
      return next
    })
    setGroupDraftBoundProxyKeys((current) => {
      if (!(normalizedGroupName in current)) return current
      const next = { ...current }
      delete next[normalizedGroupName]
      return next
    })
  }, [groupDraftBoundProxyKeys, groupDraftNotes, saveGroupNote])

  const openGroupNoteEditor = (groupName: string) => {
    if (!writesEnabled) return
    const normalized = normalizeGroupName(groupName)
    if (!normalized) return
    const existingGroup = resolveGroupSummaryForName(normalized)
    setGroupNoteError(null)
    setGroupNoteEditor({
      open: true,
      groupName: normalized,
      note: resolveGroupNoteForName(normalized),
      existing: existingGroup != null,
      boundProxyKeys: resolveGroupBoundProxyKeysForName(normalized),
    })
  }

  const closeGroupNoteEditor = () => {
    if (groupNoteBusy) return
    setGroupNoteEditor((current) => ({ ...current, open: false }))
    setGroupNoteError(null)
  }

  const handleSaveGroupNote = async () => {
    if (!writesEnabled) return
    const normalizedGroupName = normalizeGroupName(groupNoteEditor.groupName)
    if (!normalizedGroupName) return
    const normalizedNote = groupNoteEditor.note.trim()
    const normalizedBoundProxyKeys = Array.from(
      new Set(groupNoteEditor.boundProxyKeys.map((value) => value.trim()).filter((value) => value.length > 0)),
    )
    setGroupNoteError(null)

    if (!groupNoteEditor.existing) {
      setGroupDraftNotes((current) => {
        const next = { ...current }
        if (normalizedNote) {
          next[normalizedGroupName] = normalizedNote
        } else {
          delete next[normalizedGroupName]
        }
        return next
      })
      setGroupDraftBoundProxyKeys((current) => {
        const next = { ...current }
        if (normalizedBoundProxyKeys.length > 0) {
          next[normalizedGroupName] = normalizedBoundProxyKeys
        } else {
          delete next[normalizedGroupName]
        }
        return next
      })
      setGroupNoteEditor((current) => ({ ...current, open: false }))
      return
    }

    setGroupNoteBusy(true)
    try {
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
      })
      setGroupDraftNotes((current) => {
        if (!(normalizedGroupName in current)) return current
        const next = { ...current }
        delete next[normalizedGroupName]
        return next
      })
      setGroupDraftBoundProxyKeys((current) => {
        if (!(normalizedGroupName in current)) return current
        const next = { ...current }
        delete next[normalizedGroupName]
        return next
      })
      setGroupNoteEditor((current) => ({ ...current, open: false }))
    } catch (err) {
      setGroupNoteError(err instanceof Error ? err.message : String(err))
    } finally {
      setGroupNoteBusy(false)
    }
  }

  const groupFilterSuggestions = useMemo(() => {
    const suggestions = [t('accountPool.upstreamAccounts.groupFilter.all'), ...availableGroups.names]
    if (availableGroups.hasUngrouped) {
      suggestions.push(t('accountPool.upstreamAccounts.groupFilter.ungrouped'))
    }
    return suggestions
  }, [availableGroups, t])

  const {
    stats: stickyConversationStats,
    isLoading: stickyConversationLoading,
    error: stickyConversationError,
  } = useUpstreamStickyConversations(selectedId, stickyConversationLimit, Boolean(selectedId && isDetailDrawerOpen))

  const selectedDetail = detail?.id === selectedId ? detail : null
  const selected = selectedDetail ?? selectedSummary
  const selectedPlanBadge = upstreamPlanBadgeRecipe(selected?.planType)
  const detailTabIds = {
    overview: {
      tab: `${detailDrawerTabsBaseId}-overview-tab`,
      panel: `${detailDrawerTabsBaseId}-overview-panel`,
    },
    edit: {
      tab: `${detailDrawerTabsBaseId}-edit-tab`,
      panel: `${detailDrawerTabsBaseId}-edit-panel`,
    },
    routing: {
      tab: `${detailDrawerTabsBaseId}-routing-tab`,
      panel: `${detailDrawerTabsBaseId}-routing-panel`,
    },
    healthEvents: {
      tab: `${detailDrawerTabsBaseId}-health-events-tab`,
      panel: `${detailDrawerTabsBaseId}-health-events-panel`,
    },
  } as const
  const visibleAccountActionError =
    typeof selectedId === 'number' ? actionError.accountMessages[selectedId] ?? null : null
  const visibleRoutingError = actionError.routing
  const resolvedRoutingMaintenance = useMemo(
    () => resolveRoutingMaintenance(routing?.maintenance),
    [routing?.maintenance],
  )
  const parsedRoutingMaintenance = useMemo(() => {
    const primarySyncIntervalSecs = parseRoutingPositiveInteger(routingDraft.primarySyncIntervalSecs)
    const secondarySyncIntervalSecs = parseRoutingPositiveInteger(routingDraft.secondarySyncIntervalSecs)
    const priorityAvailableAccountCap = parseRoutingPositiveInteger(routingDraft.priorityAvailableAccountCap)
    if (
      primarySyncIntervalSecs == null ||
      secondarySyncIntervalSecs == null ||
      priorityAvailableAccountCap == null
    ) {
      return null
    }
    return {
      primarySyncIntervalSecs,
      secondarySyncIntervalSecs,
      priorityAvailableAccountCap,
    }
  }, [
    routingDraft.primarySyncIntervalSecs,
    routingDraft.secondarySyncIntervalSecs,
    routingDraft.priorityAvailableAccountCap,
  ])
  const routingDraftValidationError = useMemo(() => {
    if (parsedRoutingMaintenance == null) {
      return t('accountPool.upstreamAccounts.routing.validation.integerRequired')
    }
    if (parsedRoutingMaintenance.primarySyncIntervalSecs < 60) {
      return t('accountPool.upstreamAccounts.routing.validation.primaryMin')
    }
    if (parsedRoutingMaintenance.secondarySyncIntervalSecs < 60) {
      return t('accountPool.upstreamAccounts.routing.validation.secondaryMin')
    }
    if (
      parsedRoutingMaintenance.secondarySyncIntervalSecs <
      parsedRoutingMaintenance.primarySyncIntervalSecs
    ) {
      return t('accountPool.upstreamAccounts.routing.validation.secondaryAtLeastPrimary')
    }
    if (parsedRoutingMaintenance.priorityAvailableAccountCap < 1) {
      return t('accountPool.upstreamAccounts.routing.validation.priorityCapMin')
    }
    return null
  }, [parsedRoutingMaintenance, t])
  const routingHasApiKeyChange = routingDraft.apiKey.trim().length > 0
  const routingHasMaintenanceChange =
    parsedRoutingMaintenance != null &&
    (
      parsedRoutingMaintenance.primarySyncIntervalSecs !== resolvedRoutingMaintenance.primarySyncIntervalSecs ||
      parsedRoutingMaintenance.secondarySyncIntervalSecs !== resolvedRoutingMaintenance.secondarySyncIntervalSecs ||
      parsedRoutingMaintenance.priorityAvailableAccountCap !== resolvedRoutingMaintenance.priorityAvailableAccountCap
    )
  const resolvedRoutingTimeouts = routing?.timeouts ?? DEFAULT_ROUTING_TIMEOUTS
  const routingHasTimeoutChange =
    routingDraft.responsesFirstByteTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.responsesFirstByteTimeoutSecs) ||
    routingDraft.compactFirstByteTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.compactFirstByteTimeoutSecs) ||
    routingDraft.responsesStreamTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.responsesStreamTimeoutSecs) ||
    routingDraft.compactStreamTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.compactStreamTimeoutSecs)
  const routingDialogCanEdit = routingWritesEnabled && !isRoutingDialogInspectOnly
  const routingCanSave =
    routingDialogCanEdit &&
    !routingDraftValidationError &&
    (routingHasMaintenanceChange || routingHasTimeoutChange || routingHasApiKeyChange)
  const selectedRecoveryHint = resolveOauthRecoveryHint(
    selectedDetail?.kind ?? selected?.kind ?? '',
    accountHealthStatus(selectedDetail ?? selected),
    selectedDetail?.lastError ?? selected?.lastError,
  )
  const formatDuplicateReasons = (
    duplicateInfo?: UpstreamAccountDuplicateInfo | null,
  ) => {
    const reasons = duplicateInfo?.reasons ?? []
    return reasons
      .map((reason) => {
        if (reason === 'sharedChatgptAccountId') {
          return t(
            'accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId',
          )
        }
        if (reason === 'sharedChatgptUserId') {
          return t(
            'accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId',
          )
        }
        return reason
      })
      .join(' / ')
  }
  const accountEnableStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.enableStatus.${status}`)
  const accountWorkStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.workStatus.${status}`)
  const accountWorkingCountLabel = (count: number) =>
    t('accountPool.upstreamAccounts.workStatus.workingWithCount', { count })
  const accountHealthStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.healthStatus.${status}`)
  const accountSyncStateLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.syncState.${status}`)
  const accountActionLabel = (action?: string | null) => {
    if (!action) return null
    const key = `accountPool.upstreamAccounts.latestAction.actions.${action}`
    const translated = t(key)
    return translated === key ? action : translated
  }
  const accountActionSourceLabel = (source?: string | null) => {
    if (!source) return null
    const key = `accountPool.upstreamAccounts.latestAction.sources.${source}`
    const translated = t(key)
    return translated === key ? source : translated
  }
  const accountActionReasonLabel = (reason?: string | null) => {
    if (!reason) return null
    const key = `accountPool.upstreamAccounts.latestAction.reasons.${reason}`
    const translated = t(key)
    return translated === key ? reason : translated
  }
  const selectedRecentActions = selectedDetail?.recentActions ?? []
  const accountKindLabel = (kind: string) =>
    kind === 'oauth_codex'
      ? t('accountPool.upstreamAccounts.kind.oauth')
      : t('accountPool.upstreamAccounts.kind.apiKey')
  const detailDisplayNameConflict = useMemo(
    () => findDisplayNameConflict(items, draft.displayName, selectedDetail?.id ?? null),
    [draft.displayName, items, selectedDetail?.id],
  )
  const bulkRemovableTagIds = useMemo(() => {
    const removableIds = new Set<number>()
    for (const summary of Object.values(selectedAccountSummaries)) {
      for (const tag of summary.tags ?? []) {
        removableIds.add(tag.id)
      }
    }
    return Array.from(removableIds)
  }, [selectedAccountSummaries])
  const bulkRemovableTagIdSet = useMemo(
    () => new Set(bulkRemovableTagIds),
    [bulkRemovableTagIds],
  )
  const bulkUnavailableTagIds = useMemo(
    () => tagItems.filter((tag) => !bulkRemovableTagIdSet.has(tag.id)).map((tag) => tag.id),
    [bulkRemovableTagIdSet, tagItems],
  )
  const tagFieldLabels = {
    label: t('accountPool.tags.field.label'),
    add: t('accountPool.tags.field.add'),
    empty: t('accountPool.tags.field.empty'),
    searchPlaceholder: t('accountPool.tags.field.searchPlaceholder'),
    searchEmpty: t('accountPool.tags.field.searchEmpty'),
    createInline: (value: string) => t('accountPool.tags.field.createInline', { value: value || t('accountPool.tags.field.newTag') }),
    selectedFromCurrentPage: t('accountPool.tags.field.currentPage'),
    remove: t('accountPool.tags.field.remove'),
    deleteAndRemove: t('accountPool.tags.field.deleteAndRemove'),
    edit: t('accountPool.tags.field.edit'),
    createTitle: t('accountPool.tags.dialog.createTitle'),
    editTitle: t('accountPool.tags.dialog.editTitle'),
    dialogDescription: t('accountPool.tags.dialog.description'),
    name: t('accountPool.tags.dialog.name'),
    namePlaceholder: t('accountPool.tags.dialog.namePlaceholder'),
    guardEnabled: t('accountPool.tags.dialog.guardEnabled'),
    lookbackHours: t('accountPool.tags.dialog.lookbackHours'),
    maxConversations: t('accountPool.tags.dialog.maxConversations'),
    allowCutOut: t('accountPool.tags.dialog.allowCutOut'),
    allowCutIn: t('accountPool.tags.dialog.allowCutIn'),
    cancel: t('accountPool.tags.dialog.cancel'),
    save: t('accountPool.tags.dialog.save'),
    createAction: t('accountPool.tags.dialog.createAction'),
    validation: t('accountPool.tags.dialog.validation'),
  }
  const handleSelectAccount = (accountId: number) => {
    setIsDetailDrawerOpen(true)
    selectAccount(accountId)
  }
  const handleCloseDetailDrawer = () => {
    setIsDetailDrawerOpen(false)
  }

  const handleOauthLogin = async (accountId: number) => {
    navigate(`/account-pool/upstream-accounts/new?accountId=${accountId}`)
  }

  const notifyMotherChange = (updated: UpstreamAccountSummary) => {
    const nextItems = applyMotherUpdateToItems(items, updated)
    notifyMotherSwitches(items, nextItems)
  }

  const handleSave = async (source: UpstreamAccountDetail) => {
    if (source.kind === 'api_key_codex' && draftUpstreamBaseUrlError) return
    if (hasBusyAccountAction(busyAction, source.id)) return
    setActionError((current) => {
      const nextMessages = { ...current.accountMessages }
      delete nextMessages[source.id]
      return { ...current, accountMessages: nextMessages }
    })
    setBusyAction((current) => {
      const nextActions = new Set(current.accountActions)
      nextActions.add(createBusyActionKey('save', source.id))
      return { ...current, accountActions: nextActions }
    })
    try {
      const normalizedGroupName = normalizeGroupName(draft.groupName)
      const pendingGroupNote = resolvePendingGroupNoteForName(normalizedGroupName)
      const response = await saveAccount(source.id, {
        displayName: draft.displayName.trim() || undefined,
        groupName: draft.groupName.trim(),
        isMother: draft.isMother,
        note: draft.note.trim() || undefined,
        groupNote: pendingGroupNote || undefined,
        tagIds: draft.tagIds,
        upstreamBaseUrl:
          source.kind === 'api_key_codex' ? draft.upstreamBaseUrl.trim() || null : undefined,
        apiKey: source.kind === 'api_key_codex' && draft.apiKey.trim() ? draft.apiKey.trim() : undefined,
        localPrimaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localPrimaryLimit) : undefined,
        localSecondaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localSecondaryLimit) : undefined,
        localLimitUnit: source.kind === 'api_key_codex' ? draft.localLimitUnit.trim() || undefined : undefined,
      })
      let partialWarning: string | null = null
      try {
        await persistDraftGroupSettings(normalizedGroupName)
      } catch (error) {
        partialWarning = t('accountPool.upstreamAccounts.partialSuccess.savedButGroupSettingsFailed', {
          error: error instanceof Error ? error.message : String(error),
        })
      }
      notifyMotherChange(response)
      if (selectedIdRef.current === source.id) {
        setDraft((current) => ({ ...current, apiKey: '' }))
      }
      if (partialWarning) {
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: partialWarning,
          },
        }))
      }
    } catch (err) {
      setActionError((current) => ({
        ...current,
        accountMessages: {
          ...current.accountMessages,
          [source.id]: err instanceof Error ? err.message : String(err),
        },
      }))
    } finally {
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions)
        nextActions.delete(createBusyActionKey('save', source.id))
        return { ...current, accountActions: nextActions }
      })
    }
  }

  const handleSync = async (source: UpstreamAccountSummary) => {
    if (hasBusyAccountAction(busyAction, source.id)) return
    setActionError((current) => {
      const nextMessages = { ...current.accountMessages }
      delete nextMessages[source.id]
      return { ...current, accountMessages: nextMessages }
    })
    setBusyAction((current) => {
      const nextActions = new Set(current.accountActions)
      nextActions.add(createBusyActionKey('sync', source.id))
      return { ...current, accountActions: nextActions }
    })
    try {
      await runSync(source.id)
    } catch (err) {
      setActionError((current) => ({
        ...current,
        accountMessages: {
          ...current.accountMessages,
          [source.id]: err instanceof Error ? err.message : String(err),
        },
      }))
    } finally {
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions)
        nextActions.delete(createBusyActionKey('sync', source.id))
        return { ...current, accountActions: nextActions }
      })
    }
  }

  const handleToggleEnabled = async (source: UpstreamAccountSummary, enabled: boolean) => {
    if (hasBusyAccountAction(busyAction, source.id)) return
    setActionError((current) => {
      const nextMessages = { ...current.accountMessages }
      delete nextMessages[source.id]
      return { ...current, accountMessages: nextMessages }
    })
    setBusyAction((current) => {
      const nextActions = new Set(current.accountActions)
      nextActions.add(createBusyActionKey('toggle', source.id))
      return { ...current, accountActions: nextActions }
    })
    try {
      await saveAccount(source.id, { enabled })
    } catch (err) {
      setActionError((current) => ({
        ...current,
        accountMessages: {
          ...current.accountMessages,
          [source.id]: err instanceof Error ? err.message : String(err),
        },
      }))
    } finally {
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions)
        nextActions.delete(createBusyActionKey('toggle', source.id))
        return { ...current, accountActions: nextActions }
      })
    }
  }


  const handleSaveRouting = async () => {
    if (routingDraftValidationError) {
      setActionError((current) => ({ ...current, routing: routingDraftValidationError }))
      return
    }
    if (!routing) {
      setActionError((current) => ({
        ...current,
        routing: "Pool routing settings are still loading.",
      }))
      return
    }
    if (!routingWritesEnabled) {
      setActionError((current) => ({
        ...current,
        routing: "Pool routing settings are currently read-only.",
      }))
      return
    }
    const timeoutEntries: Array<[keyof PoolRoutingTimeoutSettings, string, string]> = [
      ['responsesFirstByteTimeoutSecs', t('accountPool.upstreamAccounts.routing.timeout.responsesFirstByte'), routingDraft.responsesFirstByteTimeoutSecs],
      ['compactFirstByteTimeoutSecs', t('accountPool.upstreamAccounts.routing.timeout.compactFirstByte'), routingDraft.compactFirstByteTimeoutSecs],
      ['responsesStreamTimeoutSecs', t('accountPool.upstreamAccounts.routing.timeout.responsesStream'), routingDraft.responsesStreamTimeoutSecs],
      ['compactStreamTimeoutSecs', t('accountPool.upstreamAccounts.routing.timeout.compactStream'), routingDraft.compactStreamTimeoutSecs],
    ]
    const parsedTimeouts = {} as PoolRoutingTimeoutSettings
    for (const [key, label, raw] of timeoutEntries) {
      const result = parseRoutingTimeoutValue(raw, label)
      if (!result.ok) {
        setActionError((current) => ({ ...current, routing: result.error }))
        return
      }
      parsedTimeouts[key] = result.value
    }
    setActionError((current) => ({ ...current, routing: null }))
    const trimmedApiKey = routingDraft.apiKey.trim()
    const payload: {
      apiKey?: string
      maintenance?: PoolRoutingMaintenanceSettings
      timeouts?: PoolRoutingTimeoutSettings
    } = {}
    if (routingWritesEnabled && trimmedApiKey) {
      payload.apiKey = trimmedApiKey
    }
    if (routingHasMaintenanceChange && parsedRoutingMaintenance) {
      payload.maintenance = parsedRoutingMaintenance
    }
    if (routingHasTimeoutChange) {
      payload.timeouts = parsedTimeouts
    }
    if (!payload.apiKey && !payload.maintenance && !payload.timeouts) {
      setIsRoutingDialogInspectOnly(false)
      setIsRoutingDialogOpen(false)
      return
    }
    setBusyAction((current) => ({ ...current, routing: true }))
    try {
      await saveRouting(payload)
      setRoutingDraft((current) => ({ ...current, apiKey: '' }))
      setIsRoutingDialogInspectOnly(false)
      setIsRoutingDialogOpen(false)
    } catch (err) {
      setActionError((current) => ({
        ...current,
        routing: err instanceof Error ? err.message : String(err),
      }))
    } finally {
      setBusyAction((current) => ({ ...current, routing: false }))
    }
  }

  const handleDelete = async (source: UpstreamAccountSummary) => {
    if (hasBusyAccountAction(busyAction, source.id)) return
    setIsDeleteConfirmOpen(false)
    setActionError((current) => {
      const nextMessages = { ...current.accountMessages }
      delete nextMessages[source.id]
      return { ...current, accountMessages: nextMessages }
    })
    setBusyAction((current) => {
      const nextActions = new Set(current.accountActions)
      nextActions.add(createBusyActionKey('delete', source.id))
      return { ...current, accountActions: nextActions }
    })
    try {
      await removeAccount(source.id)
      if (selectedIdRef.current === source.id) {
        setIsDetailDrawerOpen(false)
      }
    } catch (err) {
      setActionError((current) => ({
        ...current,
        accountMessages: {
          ...current.accountMessages,
          [source.id]: err instanceof Error ? err.message : String(err),
        },
      }))
    } finally {
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions)
        nextActions.delete(createBusyActionKey('delete', source.id))
        return { ...current, accountActions: nextActions }
      })
    }
  }

  const isBulkSyncRunning = bulkSyncSnapshot?.status === 'running'
  const isBulkSyncBusy = isBulkSyncRunning || isBulkSyncStarting

  const handleToggleSelectedAccount = useCallback((accountId: number, checked: boolean) => {
    setSelectedAccountIds((current) => {
      if (checked) {
        return current.includes(accountId) ? current : [...current, accountId]
      }
      return current.filter((value) => value !== accountId)
    })
  }, [])

  const handleToggleSelectAllCurrentPage = useCallback((checked: boolean) => {
    const currentPageIds = items.map((item) => item.id)
    setSelectedAccountIds((current) => {
      const next = new Set(current)
      if (checked) {
        currentPageIds.forEach((accountId) => next.add(accountId))
      } else {
        currentPageIds.forEach((accountId) => next.delete(accountId))
      }
      return Array.from(next)
    })
  }, [items])

  const closeBulkOverlays = useCallback(() => {
    setBulkGroupDialogOpen(false)
    setBulkTagsDialogOpen(false)
    setBulkDeleteDialogOpen(false)
  }, [])

  const summarizeBulkAction = useCallback((action: string, succeededCount: number, failedCount: number) => {
    setBulkActionMessage(
      t('accountPool.upstreamAccounts.bulk.resultSummary', {
        action: t(`accountPool.upstreamAccounts.bulk.actionLabel.${action}`),
        succeeded: succeededCount,
        failed: failedCount,
      }),
    )
  }, [t])

  const handleBulkAction = useCallback(
    async (
      payload: BulkUpstreamAccountActionPayload,
      options?: { clearSelection?: boolean; onSuccess?: () => void },
    ) => {
      if (selectedAccountIds.length === 0) return
      setBulkActionBusy(payload.action)
      setBulkActionError(null)
      setBulkActionMessage(null)
      try {
        const response = await runBulkAction(payload)
        summarizeBulkAction(response.action, response.succeededCount, response.failedCount)
        options?.onSuccess?.()
        if (options?.clearSelection !== false) {
          clearBulkSelection()
        }
      } catch (err) {
        setBulkActionError(err instanceof Error ? err.message : String(err))
      } finally {
        setBulkActionBusy(null)
      }
    },
    [clearBulkSelection, runBulkAction, selectedAccountIds.length, summarizeBulkAction],
  )

  const handleOpenBulkTagsDialog = useCallback((mode: 'add_tags' | 'remove_tags') => {
    setBulkTagMode(mode)
    setBulkTagIds([])
    setBulkTagsDialogOpen(true)
    setBulkActionError(null)
  }, [])

  const applyBulkSyncTerminalState = useCallback((
    nextSnapshot: BulkUpstreamAccountSyncSnapshot,
    nextCounts: BulkUpstreamAccountSyncCounts | null,
    options?: {
      error?: string | null
      status?: BulkUpstreamAccountSyncSnapshot['status']
    },
  ) => {
    const resolvedSnapshot = options?.status
      ? withBulkSyncSnapshotStatus(nextSnapshot, options.status)
      : nextSnapshot
    const resolvedCounts = resolveBulkSyncCounts(resolvedSnapshot, nextCounts)
    const shouldHide = shouldAutoHideBulkSyncProgress(resolvedSnapshot, resolvedCounts)

    closeBulkSyncEventSource()
    if (shouldHide) {
      clearBulkSyncProgress()
    } else {
      setBulkSyncSnapshot(resolvedSnapshot)
      setBulkSyncCounts(resolvedCounts)
      setBulkSyncError(options?.error ?? null)
    }
    void refresh()
  }, [clearBulkSyncProgress, closeBulkSyncEventSource, refresh])

  const handleStartBulkSync = useCallback(async () => {
    if (selectedAccountIds.length === 0 || isBulkSyncBusy) return
    setIsBulkSyncStarting(true)
    setBulkActionError(null)
    setBulkActionMessage(null)
    setBulkSyncError(null)
    closeBulkSyncEventSource()
    try {
      const created = await startBulkSyncJob({ accountIds: selectedAccountIds })
      setBulkSyncSnapshot(created.snapshot)
      setBulkSyncCounts(created.counts)
      const eventSource = createBulkUpstreamAccountSyncJobEventSource(created.jobId)
      bulkSyncEventSourceRef.current = eventSource

      eventSource.addEventListener('snapshot', (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        )
        setBulkSyncSnapshot(payload.snapshot)
        setBulkSyncCounts(payload.counts)
      })

      eventSource.addEventListener('row', (event) => {
        const payload = normalizeBulkUpstreamAccountSyncRowEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        )
        setBulkSyncCounts(payload.counts)
        setBulkSyncSnapshot((current) => {
          if (!current) return current
          return {
            ...current,
            rows: current.rows.map((row) =>
              row.accountId === payload.row.accountId ? payload.row : row,
            ),
          }
        })
      })

      const handleTerminalEvent = (
        nextSnapshot: BulkUpstreamAccountSyncSnapshot,
        nextCounts: BulkUpstreamAccountSyncCounts,
        error?: string,
        status?: BulkUpstreamAccountSyncSnapshot['status'],
      ) => {
        applyBulkSyncTerminalState(nextSnapshot, nextCounts, { error, status })
      }

      eventSource.addEventListener('completed', (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        )
        handleTerminalEvent(payload.snapshot, payload.counts, null, 'completed')
      })

      eventSource.addEventListener('cancelled', (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        )
        handleTerminalEvent(payload.snapshot, payload.counts, null, 'cancelled')
      })

      eventSource.addEventListener('failed', (event) => {
        const payload = normalizeBulkUpstreamAccountSyncFailedEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        )
        handleTerminalEvent(payload.snapshot, payload.counts, payload.error, 'failed')
      })

      eventSource.onerror = () => {
        void getBulkSyncJob(created.jobId)
          .then((latest) => {
            if (latest.snapshot.status !== 'running') {
              applyBulkSyncTerminalState(latest.snapshot, latest.counts)
              return
            }
            setBulkSyncSnapshot(latest.snapshot)
            setBulkSyncCounts(latest.counts)
          })
          .catch((err) => {
            setBulkSyncError(err instanceof Error ? err.message : String(err))
            closeBulkSyncEventSource()
          })
      }
    } catch (err) {
      setBulkSyncError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsBulkSyncStarting(false)
    }
  }, [
    applyBulkSyncTerminalState,
    closeBulkSyncEventSource,
    getBulkSyncJob,
    isBulkSyncBusy,
    refresh,
    selectedAccountIds,
    startBulkSyncJob,
  ])

  const handleCancelBulkSync = useCallback(async () => {
    if (!bulkSyncSnapshot?.jobId || bulkSyncSnapshot.status !== 'running') return
    try {
      await stopBulkSyncJob(bulkSyncSnapshot.jobId)
    } catch (err) {
      setBulkSyncError(err instanceof Error ? err.message : String(err))
    }
  }, [bulkSyncSnapshot?.jobId, bulkSyncSnapshot?.status, stopBulkSyncJob])

  const bulkSyncProgressBubble = bulkSyncSnapshot ? (
    <div className="pointer-events-none fixed inset-x-3 bottom-3 z-[65] sm:inset-x-auto sm:right-4 sm:w-[min(30rem,calc(100vw-2rem))]">
      <Card
        className={cn(
          'pointer-events-auto overflow-hidden rounded-[1.75rem] border border-base-300/85 bg-base-100/92 shadow-[0_24px_64px_rgba(15,23,42,0.28)] backdrop-blur-xl',
          bulkSyncSnapshot.status === 'running'
            ? 'ring-1 ring-primary/20'
            : 'ring-1 ring-base-300/60',
        )}
      >
        <CardHeader className="flex flex-col gap-3 border-b border-base-300/70 bg-base-100/78 pb-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="space-y-1">
            <CardTitle className="flex items-center gap-2 text-base">
              <span className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary/12 text-primary">
                {bulkSyncSnapshot.status === 'running' ? (
                  <Spinner size="sm" />
                ) : (
                  <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
                )}
              </span>
              {t('accountPool.upstreamAccounts.bulk.syncProgressTitle')}
            </CardTitle>
            <CardDescription className="text-xs leading-5 text-base-content/72">
              {t('accountPool.upstreamAccounts.bulk.syncProgressSummary', {
                completed: bulkSyncCounts?.completed ?? 0,
                total: bulkSyncCounts?.total ?? bulkSyncSnapshot.rows.length,
                succeeded: bulkSyncCounts?.succeeded ?? 0,
                failed: bulkSyncCounts?.failed ?? 0,
                skipped: bulkSyncCounts?.skipped ?? 0,
              })}
            </CardDescription>
          </div>
          {bulkSyncSnapshot.status === 'running' ? (
            <Button type="button" variant="outline" size="sm" onClick={() => void handleCancelBulkSync()}>
              {t('accountPool.upstreamAccounts.bulk.cancelSync')}
            </Button>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 rounded-full text-base-content/62 hover:text-base-content"
              aria-label={t('accountPool.upstreamAccounts.bulk.dismissSync')}
              title={t('accountPool.upstreamAccounts.bulk.dismissSync')}
              onClick={clearBulkSyncProgress}
            >
              <AppIcon name="close" className="h-4 w-4" aria-hidden />
            </Button>
          )}
        </CardHeader>
        <CardContent className="space-y-3 p-4 pt-3">
          {bulkSyncError ? (
            <Alert variant="error">
              <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{bulkSyncError}</div>
            </Alert>
          ) : null}
          <div className="max-h-[min(52vh,20rem)] space-y-2 overflow-y-auto rounded-2xl border border-base-300/80 bg-base-100/72 p-3">
            {bulkSyncSnapshot.rows.map((row) => (
              <div key={row.accountId} className="flex flex-col gap-1 rounded-xl border border-base-300/60 px-3 py-2 text-sm">
                <div className="flex items-center justify-between gap-3">
                  <span className="font-medium text-base-content">{row.displayName}</span>
                  <Badge variant={bulkSyncRowStatusVariant(row.status)}>{t(`accountPool.upstreamAccounts.bulk.rowStatus.${row.status}`)}</Badge>
                </div>
                {row.detail ? <p className="text-xs text-base-content/68">{row.detail}</p> : null}
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  ) : null

  return (
    <div className="grid gap-6">
      <section className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="surface-panel overflow-hidden">
          <div className="surface-panel-body gap-5">
            <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
              <div className="section-heading">
                <h2 className="section-title">{t('accountPool.upstreamAccounts.title')}</h2>
                <p className="section-description">{t('accountPool.upstreamAccounts.description')}</p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => void refresh()}
                  disabled={isBusyAction(busyAction, 'routing')}
                >
                  <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.upstreamAccounts.actions.refresh')}
                </Button>
                {writesEnabled ? (
                  <Button asChild>
                    <Link to="/account-pool/upstream-accounts/new">
                      <AppIcon name="plus-circle-outline" className="mr-2 h-4 w-4" aria-hidden />
                      {t('accountPool.upstreamAccounts.actions.addAccount')}
                    </Link>
                  </Button>
                ) : (
                  <Button type="button" disabled>
                    <AppIcon name="plus-circle-outline" className="mr-2 h-4 w-4" aria-hidden />
                    {t('accountPool.upstreamAccounts.actions.addAccount')}
                  </Button>
                )}
              </div>
            </div>

            {!writesEnabled ? (
              <Alert variant="warning">
                <AppIcon name="shield-key-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>
                  <p className="font-medium">{t('accountPool.upstreamAccounts.writesDisabledTitle')}</p>
                  <p className="mt-1 text-sm text-warning/90">{t('accountPool.upstreamAccounts.writesDisabledBody')}</p>
                </div>
              </Alert>
            ) : null}

            {visibleRoutingError ? (
              <Alert variant="error">
                <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>{visibleRoutingError}</div>
              </Alert>
            ) : null}

            {duplicateWarning ? (
              <Alert variant="warning">
                <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <p className="font-medium">
                    {t('accountPool.upstreamAccounts.duplicate.warningTitle', {
                      name: duplicateWarning.displayName,
                    })}
                  </p>
                  <p className="text-sm text-warning/90">
                    {t('accountPool.upstreamAccounts.duplicate.warningBody', {
                      reasons: formatDuplicateReasons({
                        peerAccountIds: duplicateWarning.peerAccountIds,
                        reasons: duplicateWarning.reasons,
                      }),
                      peers: duplicateWarning.peerAccountIds.join(', '),
                    })}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setDuplicateWarning(null)}
                >
                  {t('accountPool.upstreamAccounts.actions.dismissDuplicateWarning')}
                </Button>
              </Alert>
            ) : null}

            {postCreateWarning ? (
              <Alert variant="warning">
                <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <p className="font-medium">{t('accountPool.upstreamAccounts.partialSuccess.title')}</p>
                  <p className="text-sm text-warning/90">{postCreateWarning}</p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setPostCreateWarning(null)}
                >
                  {t('accountPool.upstreamAccounts.actions.dismissDuplicateWarning')}
                </Button>
              </Alert>
            ) : null}

            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              {metrics.map((metric) => (
                <Card key={metric.label} className="border-base-300/80 bg-base-100/72">
                  <CardContent className="flex items-center gap-4 p-5">
                    <div className={cn('flex h-12 w-12 items-center justify-center rounded-2xl bg-base-200/70', metric.accent)}>
                      <AppIcon name={metric.icon} className="h-6 w-6" aria-hidden />
                    </div>
                    <div>
                      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-base-content/55">{metric.label}</p>
                      <p className="mt-1 text-3xl font-semibold text-base-content">{metric.value}</p>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          </div>
        </div>

        <div className="grid gap-4">
          <Card className="border-base-300/80 bg-base-100/72">
            <CardHeader>
              <CardTitle>{t('accountPool.upstreamAccounts.routing.title')}</CardTitle>
              <CardDescription>{t('accountPool.upstreamAccounts.routing.description')}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="rounded-2xl border border-base-300/80 bg-base-100/75 p-3 text-sm text-base-content/75">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="metric-label">{t('accountPool.upstreamAccounts.routing.currentKey')}</p>
                    <p className="mt-2 break-all font-mono text-sm text-base-content">
                      {routing?.apiKeyConfigured ? routing?.maskedApiKey ?? t('accountPool.upstreamAccounts.routing.configured') : t('accountPool.upstreamAccounts.routing.notConfigured')}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={handleOpenRoutingDialog}
                    disabled={!routing}
                  >
                    <AppIcon name="pencil-outline" className="h-4 w-4" aria-hidden />
                    <span className="sr-only">{t('accountPool.upstreamAccounts.routing.edit')}</span>
                  </Button>
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </section>

      <section className="grid gap-6">
        <div className="surface-panel overflow-hidden">
          <div className="surface-panel-body gap-4">
            <div className="space-y-4">
              <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                <div className="section-heading">
                  <h2 className="section-title">{t('accountPool.upstreamAccounts.listTitle')}</h2>
                  <p className="section-description">{t('accountPool.upstreamAccounts.listDescription')}</p>
                </div>
                {isLoading ? (
                  <div className="flex items-center justify-start lg:justify-end">
                    <Spinner className="text-primary" />
                  </div>
                ) : null}
              </div>

              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-12">
                <label className={cn('field min-w-0', formFieldSpanVariants({ size: 'compact' }))}>
                  <span className="field-label">{t('accountPool.upstreamAccounts.workStatusFilterLabel')}</span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={workStatusFilterOptions}
                    value={workStatusFilter}
                    placeholder={t('accountPool.upstreamAccounts.workStatusFilter.all')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.workStatusFilter.searchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.workStatusFilter.empty')}
                    clearLabel={t('accountPool.upstreamAccounts.workStatusFilter.clear')}
                    ariaLabel={t('accountPool.upstreamAccounts.workStatusFilterLabel')}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleWorkStatusFilterChange}
                  />
                </label>
                <label className={cn('field min-w-0', formFieldSpanVariants({ size: 'compact' }))}>
                  <span className="field-label">{t('accountPool.upstreamAccounts.enableStatusFilterLabel')}</span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={enableStatusFilterOptions}
                    value={enableStatusFilter}
                    placeholder={t('accountPool.upstreamAccounts.enableStatusFilter.all')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.enableStatusFilter.searchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.enableStatusFilter.empty')}
                    clearLabel={t('accountPool.upstreamAccounts.enableStatusFilter.clear')}
                    ariaLabel={t('accountPool.upstreamAccounts.enableStatusFilterLabel')}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleEnableStatusFilterChange}
                  />
                </label>
                <label className={cn('field min-w-0', formFieldSpanVariants({ size: 'compact' }))}>
                  <span className="field-label">{t('accountPool.upstreamAccounts.healthStatusFilterLabel')}</span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={healthStatusFilterOptions}
                    value={healthStatusFilter}
                    placeholder={t('accountPool.upstreamAccounts.healthStatusFilter.all')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.healthStatusFilter.searchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.healthStatusFilter.empty')}
                    clearLabel={t('accountPool.upstreamAccounts.healthStatusFilter.clear')}
                    ariaLabel={t('accountPool.upstreamAccounts.healthStatusFilterLabel')}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleHealthStatusFilterChange}
                  />
                </label>
                <label className={cn('field min-w-0', formFieldSpanVariants({ size: 'wide' }))}>
                  <span className="field-label">{t('accountPool.upstreamAccounts.groupFilterLabel')}</span>
                  <UpstreamAccountGroupCombobox
                    size="filter"
                    value={groupFilterQuery}
                    suggestions={groupFilterSuggestions}
                    placeholder={t('accountPool.upstreamAccounts.groupFilterPlaceholder')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.groupFilterSearchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.groupFilterEmpty')}
                    createLabel={(value) => t('accountPool.upstreamAccounts.groupFilterUseValue', { value })}
                    ariaLabel={t('accountPool.upstreamAccounts.groupFilterLabel')}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleGroupFilterChange}
                  />
                </label>
                <label className={cn('field min-w-0', formFieldSpanVariants({ size: 'wide' }))}>
                  <span className="field-label">{t('accountPool.upstreamAccounts.tagFilterLabel')}</span>
                  <AccountTagFilterCombobox
                    size="filter"
                    tags={tagItems}
                    value={selectedTagIds}
                    placeholder={t('accountPool.upstreamAccounts.tagFilterPlaceholder')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.tagFilterSearchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.tagFilterEmpty')}
                    clearLabel={t('accountPool.upstreamAccounts.tagFilterClear')}
                    ariaLabel={t('accountPool.upstreamAccounts.tagFilterAriaLabel')}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleTagFilterChange}
                  />
                </label>
              </div>
            </div>

            {selectedAccountIds.length > 0 ? (
              <div className="rounded-[1.25rem] border border-primary/25 bg-primary/8 px-4 py-3">
                <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                  <div className="text-sm text-base-content/80">
                    {t('accountPool.upstreamAccounts.bulk.selectedCount', { count: selectedAccountIds.length })}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => void handleBulkAction({ accountIds: selectedAccountIds, action: 'enable' })}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.enable')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => void handleBulkAction({ accountIds: selectedAccountIds, action: 'disable' })}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.disable')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => {
                        setBulkGroupName('')
                        setBulkGroupDialogOpen(true)
                      }}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.setGroup')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => handleOpenBulkTagsDialog('add_tags')}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.addTags')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => handleOpenBulkTagsDialog('remove_tags')}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.removeTags')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="secondary"
                      onClick={() => void handleStartBulkSync()}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy}
                    >
                      {isBulkSyncStarting ? <Spinner size="sm" className="mr-2" /> : null}
                      {t('accountPool.upstreamAccounts.bulk.sync')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="destructive"
                      onClick={() => setBulkDeleteDialogOpen(true)}
                      disabled={Boolean(bulkActionBusy) || isBulkSyncBusy || !writesEnabled}
                    >
                      {t('accountPool.upstreamAccounts.bulk.delete')}
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      onClick={clearBulkSelection}
                      disabled={Boolean(bulkActionBusy)}
                    >
                      {t('accountPool.upstreamAccounts.bulk.clearSelection')}
                    </Button>
                  </div>
                </div>
              </div>
            ) : null}

            {bulkActionMessage ? (
              <Alert variant="success">
                <AppIcon name="check-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>{bulkActionMessage}</div>
              </Alert>
            ) : null}

            {bulkActionError ? (
              <Alert variant="error">
                <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>{bulkActionError}</div>
              </Alert>
            ) : null}

            <UpstreamAccountsTable
              items={items}
              selectedId={selectedId}
              selectedAccountIds={selectedAccountIdSet}
              onSelect={handleSelectAccount}
              onToggleSelected={handleToggleSelectedAccount}
              onToggleSelectAllCurrentPage={handleToggleSelectAllCurrentPage}
              emptyTitle={t('accountPool.upstreamAccounts.emptyTitle')}
              emptyDescription={t('accountPool.upstreamAccounts.emptyDescription')}
              labels={{
                selectPage: t('accountPool.upstreamAccounts.bulk.selectPage'),
                selectRow: (name) => t('accountPool.upstreamAccounts.bulk.selectRow', { name }),
                account: t('accountPool.upstreamAccounts.table.account'),
                sync: t('accountPool.upstreamAccounts.table.syncAndCall'),
                lastSuccess: t('accountPool.upstreamAccounts.table.lastSuccessShort'),
                lastCall: t('accountPool.upstreamAccounts.table.lastCallShort'),
                latestAction: t('accountPool.upstreamAccounts.table.latestActionShort'),
                windows: t('accountPool.upstreamAccounts.table.windows'),
                never: t('accountPool.upstreamAccounts.never'),
                primary: t('accountPool.upstreamAccounts.primaryWindowLabel'),
                primaryShort: t('accountPool.upstreamAccounts.primaryWindowShortLabel'),
                secondary: t('accountPool.upstreamAccounts.secondaryWindowLabel'),
                secondaryShort: t('accountPool.upstreamAccounts.secondaryWindowShortLabel'),
                nextReset: t('accountPool.upstreamAccounts.table.nextReset'),
                nextResetCompact: t('accountPool.upstreamAccounts.table.nextResetCompact'),
                unknown: t('accountPool.upstreamAccounts.latestAction.unknown'),
                unavailable: t('accountPool.upstreamAccounts.unavailable'),
                oauth: t('accountPool.upstreamAccounts.kind.oauth'),
                apiKey: t('accountPool.upstreamAccounts.kind.apiKey'),
                mother: t('accountPool.upstreamAccounts.mother.badge'),
                duplicate: t('accountPool.upstreamAccounts.duplicate.badge'),
                hiddenTagsA11y: (count, names) =>
                  t('accountPool.upstreamAccounts.table.hiddenTagsA11y', { count, names }),
                workStatus: accountWorkStatusLabel,
                workStatusCount: accountWorkingCountLabel,
                enableStatus: accountEnableStatusLabel,
                healthStatus: accountHealthStatusLabel,
                syncState: accountSyncStateLabel,
                action: accountActionLabel,
                compactSupport: (item) => compactSupportLabel(item.compactSupport, t),
                compactSupportHint: (item) => compactSupportHint(item.compactSupport, t),
                actionSource: (value: UpstreamAccountSummary | string | null | undefined) =>
                  accountActionSourceLabel(
                    typeof value === 'string' || value == null ? value : value.lastActionSource,
                  ),
                actionReason: (value: UpstreamAccountSummary | string | null | undefined) =>
                  accountActionReasonLabel(
                    typeof value === 'string' || value == null ? value : value.lastActionReasonCode,
                  ),
                latestActionFieldAction: t('accountPool.upstreamAccounts.latestAction.fields.action'),
                latestActionFieldSource: t('accountPool.upstreamAccounts.latestAction.fields.source'),
                latestActionFieldReason: t('accountPool.upstreamAccounts.latestAction.fields.reason'),
                latestActionFieldHttpStatus: t('accountPool.upstreamAccounts.latestAction.fields.httpStatus'),
                latestActionFieldOccurredAt: t('accountPool.upstreamAccounts.latestAction.fields.occurredAt'),
                latestActionFieldMessage: t('accountPool.upstreamAccounts.latestAction.fields.message'),
              }}
            />

            <div className="flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-center sm:justify-between">
                <div className="text-sm text-base-content/70">
                  {t('accountPool.upstreamAccounts.pagination.summary', {
                  page,
                  pageCount,
                  total: effectiveTotal,
                })}
              </div>
              <div className="flex flex-wrap items-center gap-3">
                <div className="flex items-center gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2">
                  <span className="text-sm font-medium text-base-content/65">
                    {t('accountPool.upstreamAccounts.pagination.pageSize')}
                  </span>
                  <SelectField
                    className="min-w-[7rem]"
                    value={String(pageSize)}
                    options={pageSizeOptions}
                    size="sm"
                    triggerClassName="h-10 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm"
                    aria-label={t('accountPool.upstreamAccounts.pagination.pageSize')}
                    onValueChange={(value) => handlePageSizeChange(Number(value))}
                  />
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-10 rounded-xl px-4"
                    onClick={() => setPage((current) => Math.max(1, current - 1))}
                    disabled={page <= 1}
                  >
                    {t('accountPool.upstreamAccounts.pagination.previous')}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-10 rounded-xl px-4"
                    onClick={() => setPage((current) => Math.min(pageCount, current + 1))}
                    disabled={page >= pageCount}
                  >
                    {t('accountPool.upstreamAccounts.pagination.next')}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      <Dialog open={bulkGroupDialogOpen} onOpenChange={(open) => (!bulkActionBusy ? setBulkGroupDialogOpen(open) : undefined)}>
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>{t('accountPool.upstreamAccounts.bulk.groupDialogTitle')}</DialogTitle>
              <DialogDescription>{t('accountPool.upstreamAccounts.bulk.groupDialogDescription')}</DialogDescription>
            </DialogHeader>
            <DialogCloseIcon aria-label={t('accountPool.upstreamAccounts.actions.cancel')} disabled={Boolean(bulkActionBusy)} />
          </div>
          <div className="space-y-4 px-6 py-6">
            <label className="field">
              <span className="field-label">{t('accountPool.upstreamAccounts.bulk.groupField')}</span>
              <UpstreamAccountGroupCombobox
                value={bulkGroupName}
                suggestions={groupFilterSuggestions}
                placeholder={t('accountPool.upstreamAccounts.bulk.groupPlaceholder')}
                searchPlaceholder={t('accountPool.upstreamAccounts.groupFilterSearchPlaceholder')}
                emptyLabel={t('accountPool.upstreamAccounts.groupFilterEmpty')}
                createLabel={(value) => t('accountPool.upstreamAccounts.groupFilterUseValue', { value })}
                ariaLabel={t('accountPool.upstreamAccounts.bulk.groupField')}
                onValueChange={setBulkGroupName}
              />
            </label>
          </div>
          <DialogFooter className="border-t border-base-300/80 px-6 py-5">
            <Button type="button" variant="outline" onClick={closeBulkOverlays} disabled={Boolean(bulkActionBusy)}>
              {t('accountPool.upstreamAccounts.actions.cancel')}
            </Button>
            <Button
              type="button"
              onClick={() => void handleBulkAction(
                { accountIds: selectedAccountIds, action: 'set_group', groupName: bulkGroupName.trim() },
                { onSuccess: closeBulkOverlays },
              )}
              disabled={Boolean(bulkActionBusy) || !writesEnabled}
            >
              {t('accountPool.upstreamAccounts.bulk.apply')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={bulkTagsDialogOpen} onOpenChange={(open) => (!bulkActionBusy ? setBulkTagsDialogOpen(open) : undefined)}>
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>
                {t(
                  bulkTagMode === 'add_tags'
                    ? 'accountPool.upstreamAccounts.bulk.addTagsDialogTitle'
                    : 'accountPool.upstreamAccounts.bulk.removeTagsDialogTitle',
                )}
              </DialogTitle>
              <DialogDescription>{t('accountPool.upstreamAccounts.bulk.tagsDialogDescription')}</DialogDescription>
            </DialogHeader>
            <DialogCloseIcon aria-label={t('accountPool.upstreamAccounts.actions.cancel')} disabled={Boolean(bulkActionBusy)} />
          </div>
          <div className="space-y-4 px-6 py-6">
            <label className="field">
              <span className="field-label">{t('accountPool.upstreamAccounts.bulk.tagsField')}</span>
              <AccountTagFilterCombobox
                tags={tagItems}
                value={bulkTagIds}
                prioritizedTagIds={bulkTagMode === 'remove_tags' ? bulkRemovableTagIds : undefined}
                disabledTagIds={bulkTagMode === 'remove_tags' ? bulkUnavailableTagIds : undefined}
                placeholder={t('accountPool.upstreamAccounts.bulk.tagsPlaceholder')}
                searchPlaceholder={t('accountPool.upstreamAccounts.tagFilterSearchPlaceholder')}
                emptyLabel={t('accountPool.upstreamAccounts.tagFilterEmpty')}
                clearLabel={t('accountPool.upstreamAccounts.tagFilterClear')}
                ariaLabel={t('accountPool.upstreamAccounts.bulk.tagsField')}
                onValueChange={setBulkTagIds}
              />
            </label>
          </div>
          <DialogFooter className="border-t border-base-300/80 px-6 py-5">
            <Button type="button" variant="outline" onClick={closeBulkOverlays} disabled={Boolean(bulkActionBusy)}>
              {t('accountPool.upstreamAccounts.actions.cancel')}
            </Button>
            <Button
              type="button"
              onClick={() => void handleBulkAction(
                { accountIds: selectedAccountIds, action: bulkTagMode, tagIds: bulkTagIds },
                { onSuccess: closeBulkOverlays },
              )}
              disabled={Boolean(bulkActionBusy) || bulkTagIds.length === 0 || !writesEnabled}
            >
              {t('accountPool.upstreamAccounts.bulk.apply')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={bulkDeleteDialogOpen} onOpenChange={(open) => (!bulkActionBusy ? setBulkDeleteDialogOpen(open) : undefined)}>
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>{t('accountPool.upstreamAccounts.bulk.deleteDialogTitle')}</DialogTitle>
              <DialogDescription>
                {t('accountPool.upstreamAccounts.bulk.deleteDialogDescription', { count: selectedAccountIds.length })}
              </DialogDescription>
            </DialogHeader>
            <DialogCloseIcon aria-label={t('accountPool.upstreamAccounts.actions.cancel')} disabled={Boolean(bulkActionBusy)} />
          </div>
          <DialogFooter className="px-6 py-5">
            <Button type="button" variant="outline" onClick={closeBulkOverlays} disabled={Boolean(bulkActionBusy)}>
              {t('accountPool.upstreamAccounts.actions.cancel')}
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() => void handleBulkAction(
                { accountIds: selectedAccountIds, action: 'delete' },
                { onSuccess: closeBulkOverlays },
              )}
              disabled={Boolean(bulkActionBusy) || !writesEnabled}
            >
              {t('accountPool.upstreamAccounts.actions.confirmDelete')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <RoutingSettingsDialog
        open={isRoutingDialogOpen}
        title={t('accountPool.upstreamAccounts.routing.dialogTitle')}
        description={t('accountPool.upstreamAccounts.routing.dialogDescription')}
        closeLabel={t('accountPool.upstreamAccounts.routing.close')}
        cancelLabel={t('accountPool.upstreamAccounts.actions.cancel')}
        saveLabel={t('accountPool.upstreamAccounts.routing.save')}
        apiKey={routingDraft.apiKey}
        primarySyncIntervalSecs={routingDraft.primarySyncIntervalSecs}
        secondarySyncIntervalSecs={routingDraft.secondarySyncIntervalSecs}
        priorityAvailableAccountCap={routingDraft.priorityAvailableAccountCap}
        timeoutSectionTitle={t('accountPool.upstreamAccounts.routing.timeout.sectionTitle')}
        timeoutFields={[
          {
            key: 'responsesFirstByteTimeoutSecs',
            label: t('accountPool.upstreamAccounts.routing.timeout.responsesFirstByte'),
            value: routingDraft.responsesFirstByteTimeoutSecs,
            onChange: (value) => setRoutingDraft((current) => ({ ...current, responsesFirstByteTimeoutSecs: value })),
          },
          {
            key: 'compactFirstByteTimeoutSecs',
            label: t('accountPool.upstreamAccounts.routing.timeout.compactFirstByte'),
            value: routingDraft.compactFirstByteTimeoutSecs,
            onChange: (value) => setRoutingDraft((current) => ({ ...current, compactFirstByteTimeoutSecs: value })),
          },
          {
            key: 'responsesStreamTimeoutSecs',
            label: t('accountPool.upstreamAccounts.routing.timeout.responsesStream'),
            value: routingDraft.responsesStreamTimeoutSecs,
            onChange: (value) => setRoutingDraft((current) => ({ ...current, responsesStreamTimeoutSecs: value })),
          },
          {
            key: 'compactStreamTimeoutSecs',
            label: t('accountPool.upstreamAccounts.routing.timeout.compactStream'),
            value: routingDraft.compactStreamTimeoutSecs,
            onChange: (value) => setRoutingDraft((current) => ({ ...current, compactStreamTimeoutSecs: value })),
          },
        ]}
        busy={isBusyAction(busyAction, 'routing')}
        apiKeyWritesEnabled={routingDialogCanEdit}
        timeoutWritesEnabled={routingDialogCanEdit}
        canSave={routingCanSave}
        onApiKeyChange={(value) => setRoutingDraft((current) => ({ ...current, apiKey: value }))}
        onGenerate={() => setRoutingDraft((current) => ({ ...current, apiKey: generatePoolRoutingKey() }))}
        onPrimarySyncIntervalChange={(value) =>
          setRoutingDraft((current) => ({ ...current, primarySyncIntervalSecs: value }))
        }
        onSecondarySyncIntervalChange={(value) =>
          setRoutingDraft((current) => ({ ...current, secondarySyncIntervalSecs: value }))
        }
        onPriorityAvailableAccountCapChange={(value) =>
          setRoutingDraft((current) => ({ ...current, priorityAvailableAccountCap: value }))
        }
        onClose={() => {
          setRoutingDraft(buildRoutingDraft(routing))
          setIsRoutingDialogInspectOnly(false)
          setIsRoutingDialogOpen(false)
        }}
        onSave={() => void handleSaveRouting()}
      />

      {selected ? (
        <AccountDetailDrawerShell
          open={isDetailDrawerOpen}
          labelledBy={detailDrawerTitleId}
          closeLabel={t('accountPool.upstreamAccounts.actions.closeDetails')}
          closeDisabled={isBusyAction(busyAction, 'delete', selected.id)}
          autoFocusCloseButton={!isDeleteConfirmOpen}
          onPortalContainerChange={setDetailDrawerPortalContainer}
          onClose={handleCloseDetailDrawer}
          shellClassName="max-w-[60rem]"
          header={
            <div className="space-y-4">
              <div className="space-y-3">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant={enableStatusVariant(accountEnableStatus(selected))}>
                    {accountEnableStatusLabel(accountEnableStatus(selected))}
                  </Badge>
                  <Badge variant={workStatusVariant(accountWorkStatus(selected))}>
                    {accountWorkStatusLabel(accountWorkStatus(selected))}
                  </Badge>
                  <Badge variant={syncStateVariant(accountSyncState(selected))}>
                    {accountSyncStateLabel(accountSyncState(selected))}
                  </Badge>
                  <Badge variant={healthStatusVariant(accountHealthStatus(selected))}>
                    {accountHealthStatusLabel(accountHealthStatus(selected))}
                  </Badge>
                  <Badge variant={kindVariant(selected.kind)}>{accountKindLabel(selected.kind)}</Badge>
                  {selected.planType && selectedPlanBadge ? (
                    <Badge
                      variant={selectedPlanBadge.variant}
                      className={selectedPlanBadge.className}
                      data-plan={selectedPlanBadge.dataPlan}
                    >
                      {selected.planType}
                    </Badge>
                  ) : null}
                  {selected.duplicateInfo ? (
                    <Badge variant="warning">
                      {t('accountPool.upstreamAccounts.duplicate.badge')}
                    </Badge>
                  ) : null}
                  {selected.kind === 'api_key_codex' ? (
                    <Badge variant="secondary">
                      {t('accountPool.upstreamAccounts.apiKey.localPlaceholder')}
                    </Badge>
                  ) : null}
                </div>
                <div className="section-heading">
                  <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                    {t('accountPool.upstreamAccounts.detailTitle')}
                  </p>
                  <div className="flex flex-wrap items-center gap-2">
                    <h2 id={detailDrawerTitleId} className="section-title">
                      {selected.displayName}
                    </h2>
                    {selected.isMother ? (
                      <MotherAccountBadge label={t('accountPool.upstreamAccounts.mother.badge')} />
                    ) : null}
                  </div>
                  <p className="section-description">
                    {selected.email ?? selected.maskedApiKey ?? t('accountPool.upstreamAccounts.identityUnavailable')}
                  </p>
                </div>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <div className="flex items-center gap-2 rounded-full border border-base-300/80 bg-base-100/70 px-3 py-2 text-sm">
                  <span className="text-base-content/60">{t('accountPool.upstreamAccounts.actions.enable')}</span>
                  <Switch
                    checked={selected.enabled}
                    onCheckedChange={(checked) => void handleToggleEnabled(selected, checked)}
                    disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                    aria-label={t('accountPool.upstreamAccounts.actions.enable')}
                  />
                </div>
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => void handleSync(selected)}
                  disabled={hasBusyAccountAction(busyAction, selected.id)}
                  data-testid="account-sync-button"
                >
                  {isBusyAction(busyAction, 'sync', selected.id) ? (
                    <Spinner size="sm" className="mr-2" />
                  ) : (
                    <AppIcon
                      name="timer-refresh-outline"
                      className="mr-2 h-4 w-4"
                      aria-hidden
                      data-icon-name="timer-refresh-outline"
                    />
                  )}
                  {t('accountPool.upstreamAccounts.actions.syncNow')}
                </Button>
                {selected.kind === 'oauth_codex' ? (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => void handleOauthLogin(selected.id)}
                    disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                  >
                    {isBusyAction(busyAction, 'relogin', selected.id) ? (
                      <Spinner size="sm" className="mr-2" />
                    ) : (
                      <AppIcon name="login-variant" className="mr-2 h-4 w-4" aria-hidden />
                    )}
                    {t('accountPool.upstreamAccounts.actions.relogin')}
                  </Button>
                ) : null}
                <Popover
                  open={isDeleteConfirmOpen}
                  onOpenChange={(nextOpen) => {
                    if (isBusyAction(busyAction, 'delete', selected.id) && !nextOpen) return
                    setIsDeleteConfirmOpen(nextOpen)
                  }}
                >
                  <PopoverTrigger asChild>
                    <Button
                      type="button"
                      variant="destructive"
                      disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                      aria-haspopup="dialog"
                      aria-expanded={isDeleteConfirmOpen}
                      aria-controls={isDeleteConfirmOpen ? deleteConfirmTitleId : undefined}
                    >
                      {isBusyAction(busyAction, 'delete', selected.id) ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <AppIcon name="trash-can-outline" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.delete')}
                    </Button>
                  </PopoverTrigger>
                  {detailDrawerPortalContainer ? (
                    <PopoverContent
                      container={detailDrawerPortalContainer}
                      role="alertdialog"
                      aria-modal="false"
                      aria-labelledby={deleteConfirmTitleId}
                      align="end"
                      side="top"
                      sideOffset={12}
                      className="z-[80] w-[min(22rem,calc(100vw-1.5rem))] rounded-2xl border border-base-300 bg-base-100 p-4 shadow-[0_20px_48px_rgba(15,23,42,0.24)] ring-1 ring-base-100/90"
                      onOpenAutoFocus={(event) => {
                        event.preventDefault()
                        deleteConfirmCancelRef.current?.focus()
                      }}
                      onEscapeKeyDown={(event) => {
                        event.stopPropagation()
                      }}
                    >
                      <div className="space-y-3">
                        <div className="flex items-start gap-2.5">
                          <div className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-error text-error-content shadow-sm">
                            <AppIcon name="trash-can-outline" className="h-3.5 w-3.5" aria-hidden />
                          </div>
                          <p id={deleteConfirmTitleId} className="min-w-0 break-words pr-2 text-[15px] font-semibold leading-6 text-base-content">
                            {t('accountPool.upstreamAccounts.deleteConfirmTitle', { name: selected.displayName })}
                          </p>
                        </div>
                        <div className="flex justify-end gap-2">
                          <Button
                            ref={deleteConfirmCancelRef}
                            type="button"
                            variant="secondary"
                            size="sm"
                            className="rounded-full px-3.5 font-semibold"
                            onClick={() => setIsDeleteConfirmOpen(false)}
                          >
                            {t('accountPool.upstreamAccounts.actions.cancel')}
                          </Button>
                          <Button
                            type="button"
                            variant="destructive"
                            size="sm"
                            className="rounded-full px-3.5 font-semibold shadow-sm"
                            disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                            onClick={() => void handleDelete(selected)}
                          >
                            {t('accountPool.upstreamAccounts.actions.confirmDelete')}
                          </Button>
                        </div>
                      </div>
                      <PopoverArrow className="fill-base-100 stroke-base-300 stroke-[1px]" width={18} height={10} />
                    </PopoverContent>
                  ) : null}
                </Popover>
              </div>
            </div>
          }
        >
          {isDetailLoading && !selectedDetail ? (
            <AccountDetailSkeleton />
          ) : (
            <div className="grid gap-5">
              {visibleAccountActionError ? (
                <Alert variant="error">
                  <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  <div>{visibleAccountActionError}</div>
                </Alert>
              ) : null}
              {selectedDetail ? (
                <>
                  <SegmentedControl
                    className="self-start"
                    role="tablist"
                    aria-label={t('accountPool.upstreamAccounts.detailTitle')}
                  >
                    <SegmentedControlItem
                      id={detailTabIds.overview.tab}
                      active={detailTab === 'overview'}
                      role="tab"
                      aria-selected={detailTab === 'overview'}
                      aria-controls={detailTabIds.overview.panel}
                      aria-pressed={detailTab === 'overview'}
                      onClick={() => setDetailTab('overview')}
                    >
                      {t('accountPool.upstreamAccounts.detailTabs.overview')}
                    </SegmentedControlItem>
                    <SegmentedControlItem
                      id={detailTabIds.edit.tab}
                      active={detailTab === 'edit'}
                      role="tab"
                      aria-selected={detailTab === 'edit'}
                      aria-controls={detailTabIds.edit.panel}
                      aria-pressed={detailTab === 'edit'}
                      onClick={() => setDetailTab('edit')}
                    >
                      {t('accountPool.upstreamAccounts.detailTabs.edit')}
                    </SegmentedControlItem>
                    <SegmentedControlItem
                      id={detailTabIds.routing.tab}
                      active={detailTab === 'routing'}
                      role="tab"
                      aria-selected={detailTab === 'routing'}
                      aria-controls={detailTabIds.routing.panel}
                      aria-pressed={detailTab === 'routing'}
                      onClick={() => setDetailTab('routing')}
                    >
                      {t('accountPool.upstreamAccounts.detailTabs.routing')}
                    </SegmentedControlItem>
                    <SegmentedControlItem
                      id={detailTabIds.healthEvents.tab}
                      active={detailTab === 'healthEvents'}
                      role="tab"
                      aria-selected={detailTab === 'healthEvents'}
                      aria-controls={detailTabIds.healthEvents.panel}
                      aria-pressed={detailTab === 'healthEvents'}
                      onClick={() => setDetailTab('healthEvents')}
                    >
                      {t('accountPool.upstreamAccounts.detailTabs.healthEvents')}
                    </SegmentedControlItem>
                  </SegmentedControl>

                  {detailTab === 'overview' ? (
                    <div
                      id={detailTabIds.overview.panel}
                      role="tabpanel"
                      aria-labelledby={detailTabIds.overview.tab}
                      className="grid gap-5"
                    >
                      {selectedDetail.duplicateInfo ? (
                        <Alert variant="warning">
                          <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                          <div>
                            <p className="font-medium">
                              {t('accountPool.upstreamAccounts.duplicate.badge')}
                            </p>
                            <p className="mt-1 text-sm text-warning/90">
                              {t('accountPool.upstreamAccounts.duplicate.warningBody', {
                                reasons: formatDuplicateReasons(selectedDetail.duplicateInfo),
                                peers: selectedDetail.duplicateInfo.peerAccountIds.join(', '),
                              })}
                            </p>
                          </div>
                        </Alert>
                      ) : null}
                      <div className="metric-grid">
                        <DetailField label={t('accountPool.upstreamAccounts.fields.groupName')} value={selectedDetail.groupName ?? ''} />
                        <DetailField
                          label={t('accountPool.upstreamAccounts.mother.fieldLabel')}
                          value={selectedDetail.isMother ? t('accountPool.upstreamAccounts.mother.badge') : t('accountPool.upstreamAccounts.mother.notMother')}
                        />
                        <DetailField label={t('accountPool.upstreamAccounts.fields.email')} value={selectedDetail.email ?? ''} />
                        <DetailField label={t('accountPool.upstreamAccounts.fields.accountId')} value={selectedDetail.chatgptAccountId ?? selectedDetail.maskedApiKey ?? ''} />
                        <DetailField label={t('accountPool.upstreamAccounts.fields.userId')} value={selectedDetail.chatgptUserId ?? ''} />
                        <DetailField label={t('accountPool.upstreamAccounts.fields.lastSuccessSync')} value={formatDateTime(selectedDetail.lastSuccessfulSyncAt)} />
                      </div>
                      <div className="grid gap-4 xl:grid-cols-2">
                        <UpstreamAccountUsageCard
                          title={t('accountPool.upstreamAccounts.primaryWindowLabel')}
                          description={t('accountPool.upstreamAccounts.usage.primaryDescription')}
                          window={selectedDetail.primaryWindow}
                          history={selectedDetail.history}
                          historyKey="primaryUsedPercent"
                          emptyLabel={t('accountPool.upstreamAccounts.noHistory')}
                          noteLabel={selectedDetail.kind === 'api_key_codex' ? t('accountPool.upstreamAccounts.apiKey.localPlaceholder') : undefined}
                        />
                        <UpstreamAccountUsageCard
                          title={t('accountPool.upstreamAccounts.secondaryWindowLabel')}
                          description={t('accountPool.upstreamAccounts.usage.secondaryDescription')}
                          window={selectedDetail.secondaryWindow}
                          history={selectedDetail.history}
                          historyKey="secondaryUsedPercent"
                          emptyLabel={t('accountPool.upstreamAccounts.noHistory')}
                          noteLabel={selectedDetail.kind === 'api_key_codex' ? t('accountPool.upstreamAccounts.apiKey.localPlaceholder') : undefined}
                          accentClassName="text-secondary"
                        />
                      </div>
                    </div>
                  ) : null}

                  {detailTab === 'edit' ? (
                    <div
                      id={detailTabIds.edit.panel}
                      role="tabpanel"
                      aria-labelledby={detailTabIds.edit.tab}
                    >
                      <Card className="border-base-300/80 bg-base-100/72">
                        <CardHeader>
                          <CardTitle>{t('accountPool.upstreamAccounts.editTitle')}</CardTitle>
                          <CardDescription>{t('accountPool.upstreamAccounts.editDescription')}</CardDescription>
                        </CardHeader>
                        <CardContent className="grid gap-4 md:grid-cols-2">
                          <label className="field md:col-span-2">
                            <span className="field-label">{t('accountPool.upstreamAccounts.fields.displayName')}</span>
                            <div className="relative">
                              <Input
                                name="detailDisplayName"
                                value={draft.displayName}
                                aria-invalid={detailDisplayNameConflict != null}
                                onChange={(event) =>
                                  setDraft((current) => ({
                                    ...current,
                                    displayName: event.target.value,
                                  }))
                                }
                              />
                              {detailDisplayNameConflict ? (
                                <FloatingFieldError
                                  message={t('accountPool.upstreamAccounts.validation.displayNameDuplicate')}
                                />
                              ) : null}
                            </div>
                          </label>
                          <label className="field md:col-span-2">
                            <span className="field-label">{t('accountPool.upstreamAccounts.fields.groupName')}</span>
                            <div className="flex items-center gap-2">
                              <UpstreamAccountGroupCombobox
                                name="detailGroupName"
                                value={draft.groupName}
                                suggestions={availableGroups.names}
                                placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                                searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                                emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                                createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                                onValueChange={(value) => setDraft((current) => ({ ...current, groupName: value }))}
                                className="min-w-0 flex-1"
                              />
                              <Button
                                type="button"
                                size="icon"
                                variant={hasGroupSettings(draft.groupName) ? 'secondary' : 'outline'}
                                className="shrink-0 rounded-full"
                                aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                onClick={() => openGroupNoteEditor(draft.groupName)}
                                disabled={!writesEnabled || !normalizeGroupName(draft.groupName)}
                              >
                                <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
                              </Button>
                            </div>
                          </label>
                          <div className="md:col-span-2">
                            <MotherAccountToggle
                              checked={draft.isMother}
                              disabled={!writesEnabled}
                              label={t('accountPool.upstreamAccounts.mother.toggleLabel')}
                              description={t('accountPool.upstreamAccounts.mother.toggleDescription')}
                              onToggle={() => setDraft((current) => ({ ...current, isMother: !current.isMother }))}
                            />
                          </div>
                          <label className="field md:col-span-2">
                            <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                            <textarea
                              className="min-h-24 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                              name="detailNote"
                              value={draft.note}
                              onChange={(event) => setDraft((current) => ({ ...current, note: event.target.value }))}
                            />
                          </label>
                          <div className="md:col-span-2">
                            <AccountTagField
                              tags={tagItems}
                              selectedTagIds={draft.tagIds}
                              writesEnabled={writesEnabled}
                              pageCreatedTagIds={pageCreatedTagIds}
                              labels={tagFieldLabels}
                              onChange={(tagIds) => setDraft((current) => ({ ...current, tagIds }))}
                              onCreateTag={handleCreateTag}
                              onUpdateTag={updateTag}
                              onDeleteTag={handleDeleteTag}
                            />
                          </div>
                          {selectedDetail.kind === 'api_key_codex' ? (
                            <>
                              <label className="field">
                                <span className="field-label">{t('accountPool.upstreamAccounts.fields.primaryLimit')}</span>
                                <Input
                                  name="detailPrimaryLimit"
                                  value={draft.localPrimaryLimit}
                                  onChange={(event) => setDraft((current) => ({ ...current, localPrimaryLimit: event.target.value }))}
                                />
                              </label>
                              <label className="field">
                                <span className="field-label">{t('accountPool.upstreamAccounts.fields.secondaryLimit')}</span>
                                <Input
                                  name="detailSecondaryLimit"
                                  value={draft.localSecondaryLimit}
                                  onChange={(event) => setDraft((current) => ({ ...current, localSecondaryLimit: event.target.value }))}
                                />
                              </label>
                              <label className="field">
                                <span className="field-label">{t('accountPool.upstreamAccounts.fields.limitUnit')}</span>
                                <Input
                                  name="detailLimitUnit"
                                  value={draft.localLimitUnit}
                                  onChange={(event) => setDraft((current) => ({ ...current, localLimitUnit: event.target.value }))}
                                />
                              </label>
                              <label className="field">
                                <FormFieldFeedback
                                  label={t('accountPool.upstreamAccounts.fields.upstreamBaseUrl')}
                                  message={draftUpstreamBaseUrlError}
                                  messageClassName="md:max-w-[min(20rem,calc(100%-8rem))]"
                                />
                                <div className="relative">
                                  <Input
                                    name="detailUpstreamBaseUrl"
                                    value={draft.upstreamBaseUrl}
                                    onChange={(event) => setDraft((current) => ({ ...current, upstreamBaseUrl: event.target.value }))}
                                    placeholder={t('accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder')}
                                    aria-invalid={draftUpstreamBaseUrlError ? 'true' : 'false'}
                                    className={cn(draftUpstreamBaseUrlError ? 'border-error/70 focus-visible:ring-error' : '')}
                                  />
                                </div>
                              </label>
                              <label className="field">
                                <span className="field-label">{t('accountPool.upstreamAccounts.fields.rotateApiKey')}</span>
                                <Input
                                  name="detailRotateApiKey"
                                  value={draft.apiKey}
                                  onChange={(event) => setDraft((current) => ({ ...current, apiKey: event.target.value }))}
                                  placeholder={t('accountPool.upstreamAccounts.fields.rotateApiKeyPlaceholder')}
                                />
                              </label>
                            </>
                          ) : null}
                          <div className="md:col-span-2 flex justify-end">
                            <Button
                              type="button"
                              onClick={() => void handleSave(selectedDetail)}
                              disabled={
                                hasBusyAccountAction(busyAction, selectedDetail.id) ||
                                !writesEnabled ||
                                detailDisplayNameConflict != null ||
                                (selectedDetail.kind === 'api_key_codex' && Boolean(draftUpstreamBaseUrlError))
                              }
                            >
                              {isBusyAction(busyAction, 'save', selectedDetail.id) ? (
                                <Spinner size="sm" className="mr-2" />
                              ) : (
                                <AppIcon name="content-save-outline" className="mr-2 h-4 w-4" aria-hidden />
                              )}
                              {t('accountPool.upstreamAccounts.actions.save')}
                            </Button>
                          </div>
                        </CardContent>
                      </Card>
                    </div>
                  ) : null}

                  {detailTab === 'routing' ? (
                    <div
                      id={detailTabIds.routing.panel}
                      role="tabpanel"
                      aria-labelledby={detailTabIds.routing.tab}
                      className="grid gap-5"
                    >
                      <EffectiveRoutingRuleCard
                        rule={selectedDetail.effectiveRoutingRule}
                        labels={{
                          title: t('accountPool.upstreamAccounts.effectiveRule.title'),
                          description: t('accountPool.upstreamAccounts.effectiveRule.description'),
                          noTags: t('accountPool.upstreamAccounts.effectiveRule.noTags'),
                          guardEnabled: t('accountPool.upstreamAccounts.effectiveRule.guardEnabled'),
                          guardDisabled: t('accountPool.upstreamAccounts.effectiveRule.guardDisabled'),
                          allowCutOut: t('accountPool.upstreamAccounts.effectiveRule.allowCutOut'),
                          denyCutOut: t('accountPool.upstreamAccounts.effectiveRule.denyCutOut'),
                          allowCutIn: t('accountPool.upstreamAccounts.effectiveRule.allowCutIn'),
                          denyCutIn: t('accountPool.upstreamAccounts.effectiveRule.denyCutIn'),
                          sourceTags: t('accountPool.upstreamAccounts.effectiveRule.sourceTags'),
                          guardRule: (hours, count) => t('accountPool.upstreamAccounts.effectiveRule.guardRule', { hours, count }),
                          allGuardsApply: t('accountPool.upstreamAccounts.effectiveRule.allGuardsApply'),
                        }}
                      />

                      <Card className="border-base-300/80 bg-base-100/72">
                        <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
                          <div>
                            <CardTitle>{t('accountPool.upstreamAccounts.stickyConversations.title')}</CardTitle>
                            <CardDescription>{t('accountPool.upstreamAccounts.stickyConversations.description')}</CardDescription>
                          </div>
                          <SelectField
                            label={t('accountPool.upstreamAccounts.stickyConversations.limitLabel')}
                            className="w-36"
                            name="stickyConversationLimit"
                            size="sm"
                            value={String(stickyConversationLimit)}
                            options={STICKY_CONVERSATION_LIMIT_OPTIONS.map((value) => ({
                              value: String(value),
                              label: t('accountPool.upstreamAccounts.stickyConversations.limitOption', { count: value }),
                            }))}
                            onValueChange={(value) => setStickyConversationLimit(Number(value))}
                          />
                        </CardHeader>
                        <CardContent>
                          <StickyKeyConversationTable
                            stats={stickyConversationStats}
                            isLoading={stickyConversationLoading}
                            error={stickyConversationError}
                          />
                        </CardContent>
                      </Card>
                    </div>
                  ) : null}

                  {detailTab === 'healthEvents' ? (
                    <div
                      id={detailTabIds.healthEvents.panel}
                      role="tabpanel"
                      aria-labelledby={detailTabIds.healthEvents.tab}
                      className="grid gap-5"
                    >
                      <Card className="border-base-300/80 bg-base-100/72">
                        <CardHeader>
                          <CardTitle>{t('accountPool.upstreamAccounts.healthTitle')}</CardTitle>
                          <CardDescription>{t('accountPool.upstreamAccounts.healthDescription')}</CardDescription>
                        </CardHeader>
                        <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                          <DetailField label={t('accountPool.upstreamAccounts.fields.lastSyncedAt')} value={formatDateTime(selectedDetail.lastSyncedAt)} />
                          <DetailField label={t('accountPool.upstreamAccounts.fields.lastRefreshedAt')} value={formatDateTime(selectedDetail.lastRefreshedAt)} />
                          <DetailField label={t('accountPool.upstreamAccounts.fields.tokenExpiresAt')} value={formatDateTime(selectedDetail.tokenExpiresAt)} />
                          <DetailField
                            label={t('accountPool.upstreamAccounts.fields.compactSupport')}
                            value={
                              selectedDetail.compactSupport?.status === 'supported'
                                ? t('accountPool.upstreamAccounts.compactSupport.status.supported')
                                : selectedDetail.compactSupport?.status === 'unsupported'
                                  ? t('accountPool.upstreamAccounts.compactSupport.status.unsupported')
                                  : t('accountPool.upstreamAccounts.compactSupport.status.unknown')
                            }
                          />
                          <DetailField
                            label={t('accountPool.upstreamAccounts.fields.credits')}
                            value={selectedDetail.credits?.balance ? `${selectedDetail.credits.balance}` : selectedDetail.credits?.unlimited ? t('accountPool.upstreamAccounts.unlimited') : t('accountPool.upstreamAccounts.unavailable')}
                          />
                          <DetailField
                            label={t('accountPool.upstreamAccounts.fields.compactObservedAt')}
                            value={formatDateTime(selectedDetail.compactSupport?.observedAt)}
                          />
                          <DetailField
                            label={t('accountPool.upstreamAccounts.fields.compactReason')}
                            value={selectedDetail.compactSupport?.reason ?? t('accountPool.upstreamAccounts.unavailable')}
                          />
                          <div className="md:col-span-2 xl:col-span-4 rounded-[1.2rem] border border-base-300/80 bg-base-100/75 p-4">
                            {selectedRecoveryHint ? (
                              <Alert variant="warning" className="mb-4">
                                <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                                <div>
                                  <p className="font-semibold text-warning">
                                    {t(selectedRecoveryHint.titleKey)}
                                  </p>
                                  <p className="mt-1 text-sm text-warning/90">
                                    {t(selectedRecoveryHint.bodyKey)}
                                  </p>
                                </div>
                              </Alert>
                            ) : null}
                            <p className="metric-label">{t('accountPool.upstreamAccounts.latestAction.title')}</p>
                            {selectedDetail.lastAction ? (
                              <div className="mt-3 grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.action')}
                                  value={accountActionLabel(selectedDetail.lastAction) ?? t('accountPool.upstreamAccounts.latestAction.empty')}
                                />
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.source')}
                                  value={
                                    accountActionSourceLabel(selectedDetail.lastActionSource)
                                    ?? t('accountPool.upstreamAccounts.latestAction.unknown')
                                  }
                                />
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.reason')}
                                  value={
                                    accountActionReasonLabel(selectedDetail.lastActionReasonCode)
                                    ?? t('accountPool.upstreamAccounts.latestAction.unknown')
                                  }
                                />
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.httpStatus')}
                                  value={
                                    Number.isFinite(selectedDetail.lastActionHttpStatus ?? NaN)
                                      ? `HTTP ${selectedDetail.lastActionHttpStatus}`
                                      : t('accountPool.upstreamAccounts.unavailable')
                                  }
                                />
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.occurredAt')}
                                  value={formatDateTime(selectedDetail.lastActionAt)}
                                />
                                <DetailField
                                  label={t('accountPool.upstreamAccounts.latestAction.fields.invokeId')}
                                  value={selectedDetail.lastActionInvokeId ?? t('accountPool.upstreamAccounts.unavailable')}
                                />
                                <div className="metric-cell md:col-span-2 xl:col-span-3">
                                  <p className="metric-label">{t('accountPool.upstreamAccounts.latestAction.fields.message')}</p>
                                  <p className="mt-2 break-words text-sm leading-6 text-base-content/80">
                                    {selectedDetail.lastActionReasonMessage ?? selectedDetail.lastError ?? t('accountPool.upstreamAccounts.noError')}
                                  </p>
                                </div>
                              </div>
                            ) : (
                              <p className="mt-2 text-sm leading-6 text-base-content/75">
                                {t('accountPool.upstreamAccounts.latestAction.empty')}
                              </p>
                            )}
                          </div>
                        </CardContent>
                      </Card>

                      <Card className="border-base-300/80 bg-base-100/72">
                        <CardHeader>
                          <CardTitle>{t('accountPool.upstreamAccounts.recentActions.title')}</CardTitle>
                          <CardDescription>{t('accountPool.upstreamAccounts.recentActions.description')}</CardDescription>
                        </CardHeader>
                        <CardContent>
                          {selectedRecentActions.length === 0 ? (
                            <p className="text-sm leading-6 text-base-content/68">
                              {t('accountPool.upstreamAccounts.recentActions.empty')}
                            </p>
                          ) : (
                            <div className="space-y-2">
                              {selectedRecentActions.map((actionEvent) => (
                                <div
                                  key={actionEvent.id}
                                  className="rounded-[1rem] border border-base-300/70 bg-base-100/70 p-3"
                                >
                                  <div className="flex flex-wrap items-center gap-2">
                                    <Badge variant="secondary">
                                      {accountActionLabel(actionEvent.action) ?? t('accountPool.upstreamAccounts.latestAction.unknown')}
                                    </Badge>
                                    <Badge variant="secondary">
                                      {accountActionSourceLabel(actionEvent.source) ?? t('accountPool.upstreamAccounts.latestAction.unknown')}
                                    </Badge>
                                    {actionEvent.reasonCode ? (
                                      <Badge variant="secondary">
                                        {accountActionReasonLabel(actionEvent.reasonCode)}
                                      </Badge>
                                    ) : null}
                                    {Number.isFinite(actionEvent.httpStatus ?? NaN) ? (
                                      <Badge variant="secondary">{`HTTP ${actionEvent.httpStatus}`}</Badge>
                                    ) : null}
                                    <span className="text-xs text-base-content/55">
                                      {formatDateTime(actionEvent.occurredAt)}
                                    </span>
                                  </div>
                                  {actionEvent.reasonMessage ? (
                                    <p className="mt-2 text-sm leading-6 text-base-content/75">
                                      {actionEvent.reasonMessage}
                                    </p>
                                  ) : null}
                                  {actionEvent.invokeId ? (
                                    <p className="mt-2 text-xs text-base-content/55">
                                      {t('accountPool.upstreamAccounts.latestAction.fields.invokeId')}: {actionEvent.invokeId}
                                    </p>
                                  ) : null}
                                </div>
                              ))}
                            </div>
                          )}
                        </CardContent>
                      </Card>
                    </div>
                  ) : null}
                </>
              ) : null}
            </div>
          )}
        </AccountDetailDrawerShell>
      ) : null}

      <UpstreamAccountGroupNoteDialog
        open={groupNoteEditor.open}
        container={detailDrawerPortalContainer}
        groupName={groupNoteEditor.groupName}
        note={groupNoteEditor.note}
        boundProxyKeys={groupNoteEditor.boundProxyKeys}
        availableProxyNodes={forwardProxyNodes}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null)
          setGroupNoteEditor((current) => ({ ...current, note: value }))
        }}
        onBoundProxyKeysChange={(value) => {
          setGroupNoteError(null)
          setGroupNoteEditor((current) => ({ ...current, boundProxyKeys: value }))
        }}
        onClose={closeGroupNoteEditor}
        onSave={() => void handleSaveGroupNote()}
        title={t('accountPool.upstreamAccounts.groupNotes.dialogTitle')}
        existingDescription={t('accountPool.upstreamAccounts.groupNotes.existingDescription')}
        draftDescription={t('accountPool.upstreamAccounts.groupNotes.draftDescription')}
        noteLabel={t('accountPool.upstreamAccounts.fields.note')}
        notePlaceholder={t('accountPool.upstreamAccounts.groupNotes.notePlaceholder')}
        cancelLabel={t('accountPool.upstreamAccounts.actions.cancel')}
        saveLabel={t('accountPool.upstreamAccounts.actions.save')}
        closeLabel={t('accountPool.upstreamAccounts.actions.closeDetails')}
        existingBadgeLabel={t('accountPool.upstreamAccounts.groupNotes.badges.existing')}
        draftBadgeLabel={t('accountPool.upstreamAccounts.groupNotes.badges.draft')}
        proxyBindingsLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.label')}
        proxyBindingsHint={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.hint')}
        proxyBindingsAutomaticLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.automatic')}
        proxyBindingsEmptyLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.empty')}
        proxyBindingsMissingLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.missing')}
        proxyBindingsUnavailableLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.unavailable')}
        proxyBindingsChartLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel')}
        proxyBindingsChartSuccessLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess')}
        proxyBindingsChartFailureLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure')}
        proxyBindingsChartEmptyLabel={t('accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty')}
        proxyBindingsChartTotalLabel={t('live.proxy.table.requestTooltip.total')}
        proxyBindingsChartAriaLabel={t('live.proxy.table.requestTrendAria')}
        proxyBindingsChartInteractionHint={t('live.chart.tooltip.instructions')}
        proxyBindingsChartLocaleTag={locale === 'zh' ? 'zh-CN' : 'en-US'}
      />

      {listError ? (
        <Alert variant="warning">
          <AppIcon name="information-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <div>{listError}</div>
        </Alert>
      ) : null}

      {detailError ? (
        <Alert variant="warning">
          <AppIcon name="information-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <div>{detailError}</div>
        </Alert>
      ) : null}

      {bulkSyncProgressBubble}
    </div>
  )
}
