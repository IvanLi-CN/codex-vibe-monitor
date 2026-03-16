import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AppIcon, type AppIconName } from '../../components/AppIcon'
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
import { MotherAccountBadge, MotherAccountToggle } from '../../components/MotherAccountToggle'
import { Spinner } from '../../components/ui/spinner'
import { Switch } from '../../components/ui/switch'
import { AccountTagField } from '../../components/AccountTagField'
import { EffectiveRoutingRuleCard } from '../../components/EffectiveRoutingRuleCard'
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
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
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
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

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

const STICKY_CONVERSATION_LIMIT_OPTIONS = [20, 50, 100] as const

type UpstreamAccountsLocationState = {
  selectedAccountId?: number
  openDetail?: boolean
  openDeleteConfirm?: boolean
  duplicateWarning?: {
    accountId: number
    displayName: string
    peerAccountIds: number[]
    reasons: string[]
  } | null
}

type GroupNoteEditorState = {
  open: boolean
  groupName: string
  note: string
  existing: boolean
}

type OauthRecoveryHint = {
  titleKey: string
  bodyKey: string
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

function buildRoutingDraft(maskedApiKey?: string | null) {
  return {
    apiKey: '',
    maskedApiKey: maskedApiKey ?? null,
  }
}

function statusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'active') return 'success'
  if (status === 'syncing') return 'warning'
  if (status === 'error' || status === 'needs_reauth') return 'error'
  return 'secondary'
}

function kindVariant(kind: string): 'secondary' | 'success' {
  return kind === 'oauth_codex' ? 'success' : 'secondary'
}

function isOauthBridgeUnavailableError(lastError?: string | null) {
  const normalized = lastError?.toLocaleLowerCase() ?? ''
  return (
    normalized.includes('oauth bridge') &&
    (normalized.includes('unavailable') || normalized.includes('connection refused'))
  )
}

function isOauthBridgeExchangeError(lastError?: string | null) {
  const normalized = lastError?.toLocaleLowerCase() ?? ''
  return normalized.includes('oauth bridge token exchange failed')
}

function isOauthBridgeUpstreamRejectedError(lastError?: string | null) {
  const normalized = lastError?.toLocaleLowerCase() ?? ''
  return normalized.includes('oauth bridge upstream') || normalized.includes('upstream rejected request')
}

function resolveOauthRecoveryHint(
  kind: string,
  status: string,
  lastError?: string | null,
): OauthRecoveryHint | null {
  if (kind !== 'oauth_codex') return null
  if (isOauthBridgeUnavailableError(lastError)) {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.bridgeUnavailableTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.bridgeUnavailableBody',
    }
  }
  if (isOauthBridgeExchangeError(lastError)) {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.bridgeExchangeTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.bridgeExchangeBody',
    }
  }
  if (isOauthBridgeUpstreamRejectedError(lastError)) {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.bridgeUpstreamTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.bridgeUpstreamBody',
    }
  }
  if (status === 'needs_reauth') {
    return {
      titleKey: 'accountPool.upstreamAccounts.hints.reauthTitle',
      bodyKey: 'accountPool.upstreamAccounts.hints.reauthBody',
    }
  }
  return null
}

function resolveDisplayedStatus(status: string, lastError?: string | null) {
  if (
    status === 'needs_reauth' &&
    (isOauthBridgeUnavailableError(lastError) ||
      isOauthBridgeExchangeError(lastError) ||
      isOauthBridgeUpstreamRejectedError(lastError))
  ) {
    return 'error'
  }
  return status
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

function AccountDetailDrawer({
  open,
  title,
  subtitle,
  closeLabel,
  closeDisabled = false,
  autoFocusCloseButton = true,
  onPortalContainerChange,
  onClose,
  children,
}: {
  open: boolean
  title: string
  subtitle?: string
  closeLabel: string
  closeDisabled?: boolean
  autoFocusCloseButton?: boolean
  onPortalContainerChange?: (node: HTMLElement | null) => void
  onClose: () => void
  children: ReactNode
}) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    if (!open || typeof document === 'undefined') return undefined

    const previousOverflow = document.body.style.overflow
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (closeDisabled) return
        onClose()
      }
    }

    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', handleKeyDown)
    const focusTimer = autoFocusCloseButton
      ? window.setTimeout(() => closeButtonRef.current?.focus(), 0)
      : null

    return () => {
      if (focusTimer != null) {
        window.clearTimeout(focusTimer)
      }
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [autoFocusCloseButton, closeDisabled, onClose, open])

  if (!open || typeof document === 'undefined') return null

  return createPortal(
    <div className="fixed inset-0 z-[70]">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-neutral/50 backdrop-blur-sm"
        onClick={closeDisabled ? undefined : onClose}
      />
      <div className="absolute inset-y-0 right-0 flex w-full justify-end pl-4 sm:pl-8">
        <section
          ref={onPortalContainerChange}
          role="dialog"
          aria-modal="true"
          aria-labelledby="upstream-account-detail-title"
          className="drawer-shell flex h-full w-full max-w-[60rem] flex-col"
        >
          <div className="drawer-header px-5 py-4 sm:px-6">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                  {subtitle}
                </p>
                <h2 id="upstream-account-detail-title" className="truncate text-xl font-semibold text-base-content">
                  {title}
                </h2>
              </div>
              <Button
                ref={closeButtonRef}
                type="button"
                variant="ghost"
                size="icon"
                onClick={onClose}
                disabled={closeDisabled}
              >
                <AppIcon name="close" className="h-5 w-5" aria-hidden />
                <span className="sr-only">{closeLabel}</span>
              </Button>
            </div>
          </div>
          <div className="drawer-body min-h-0 flex-1 overflow-y-auto px-5 py-5 sm:px-6 sm:py-6">{children}</div>
        </section>
      </div>
    </div>,
    document.body,
  )
}

function RoutingSettingsDialog({
  open,
  title,
  description,
  closeLabel,
  apiKeyLabel,
  generateLabel,
  apiKeyPlaceholder,
  cancelLabel,
  saveLabel,
  apiKey,
  busy,
  writesEnabled,
  onApiKeyChange,
  onGenerate,
  onClose,
  onSave,
}: {
  open: boolean
  title: string
  description: string
  closeLabel: string
  apiKeyLabel: string
  generateLabel: string
  apiKeyPlaceholder: string
  cancelLabel: string
  saveLabel: string
  apiKey: string
  busy: boolean
  writesEnabled: boolean
  onApiKeyChange: (value: string) => void
  onGenerate: () => void
  onClose: () => void
  onSave: () => void
}) {
  const inputRef = useRef<HTMLInputElement | null>(null)
  const inputId = 'pool-routing-secret-input'

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}>
      <DialogContent
        className="p-0"
        onOpenAutoFocus={(event) => {
          event.preventDefault()
          inputRef.current?.focus()
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
        <div className="space-y-4 px-6 py-6">
          <div className="field">
            <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
              <label htmlFor={inputId} className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                {apiKeyLabel}
              </label>
              <Button type="button" variant="outline" size="sm" onClick={onGenerate} disabled={busy || !writesEnabled}>
                <AppIcon name="auto-fix" className="mr-2 h-4 w-4" aria-hidden />
                {generateLabel}
              </Button>
            </div>
            <Input
              id={inputId}
              ref={inputRef}
              name="poolRoutingSecret"
              type="text"
              value={apiKey}
              onChange={(event) => onApiKeyChange(event.target.value)}
              placeholder={apiKeyPlaceholder}
              autoComplete="off"
              autoCorrect="off"
              autoCapitalize="none"
              spellCheck={false}
              data-1p-ignore="true"
              data-lpignore="true"
              className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono placeholder:text-base-content/58"
            />
          </div>
        </div>
        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy || !writesEnabled}>
            {busy ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="key-chain-variant" className="mr-2 h-4 w-4" aria-hidden />}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

export default function UpstreamAccountsPage() {
  const { t } = useTranslation()
  const location = useLocation()
  const navigate = useNavigate()
  const {
    items,
    groups = [],
    writesEnabled,
    selectedId,
    selectedSummary,
    detail,
    isLoading,
    isDetailLoading,
    error,
    selectAccount,
    refresh,
    saveAccount,
    runSync,
    removeAccount,
    routing,
    saveRouting,
    saveGroupNote,
  } = useUpstreamAccounts()
  const { items: tagItems, createTag, updateTag, deleteTag } = usePoolTags()
  const notifyMotherSwitches = useMotherSwitchNotifications()

  const [draft, setDraft] = useState<AccountDraft>(buildDraft(null))
  const [routingDraft, setRoutingDraft] = useState(() => buildRoutingDraft(null))
  const [pageActionError, setPageActionError] = useState<string | null>(null)
  const [detailActionError, setDetailActionError] = useState<string | null>(null)
  const [busyAction, setBusyAction] = useState<string | null>(null)
  const [isDetailDrawerOpen, setIsDetailDrawerOpen] = useState(false)
  const [isRoutingDialogOpen, setIsRoutingDialogOpen] = useState(false)
  const [isDeleteConfirmOpen, setIsDeleteConfirmOpen] = useState(false)
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([])
  const [groupFilterQuery, setGroupFilterQuery] = useState('')
  const [stickyConversationLimit, setStickyConversationLimit] = useState<number>(50)
  const [groupDraftNotes, setGroupDraftNotes] = useState<Record<string, string>>({})
  const [duplicateWarning, setDuplicateWarning] =
    useState<UpstreamAccountsLocationState['duplicateWarning']>(null)
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupNoteEditorState>({
    open: false,
    groupName: '',
    note: '',
    existing: false,
  })
  const [groupNoteBusy, setGroupNoteBusy] = useState(false)
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null)
  const deleteConfirmCancelRef = useRef<HTMLButtonElement | null>(null)
  const [detailDrawerPortalContainer, setDetailDrawerPortalContainer] = useState<HTMLElement | null>(null)
  const skipNextDeleteConfirmResetRef = useRef(false)
  const deleteConfirmTitleId = useId()

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
    setDetailActionError(null)
    if (skipNextDeleteConfirmResetRef.current) {
      skipNextDeleteConfirmResetRef.current = false
      return
    }
    setIsDeleteConfirmOpen(false)
  }, [selectedId, isDetailDrawerOpen])

  useEffect(() => {
    setRoutingDraft(buildRoutingDraft(routing?.maskedApiKey))
  }, [routing?.maskedApiKey])

  useEffect(() => {
    if (!writesEnabled) {
      setIsRoutingDialogOpen(false)
      setIsDeleteConfirmOpen(false)
    }
  }, [writesEnabled])

  useEffect(() => {
    setGroupDraftNotes((current) => {
      const nextEntries = Object.entries(current).filter(([groupName]) => !isExistingGroup(groups, groupName))
      if (nextEntries.length === Object.keys(current).length) {
        return current
      }
      return Object.fromEntries(nextEntries)
    })
  }, [groups])

  useEffect(() => {
    const state = location.state as UpstreamAccountsLocationState | null
    if (!state?.selectedAccountId) return

    skipNextDeleteConfirmResetRef.current = Boolean(state.openDeleteConfirm)
    selectAccount(state.selectedAccountId)
    setIsDetailDrawerOpen(Boolean(state.openDetail))
    setIsDeleteConfirmOpen(Boolean(state.openDeleteConfirm))
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
    const displayedStatuses = items.map((item) => resolveDisplayedStatus(item.status, item.lastError))
    const oauthCount = items.filter((item) => item.kind === 'oauth_codex').length
    const apiKeyCount = items.filter((item) => item.kind === 'api_key_codex').length
    const needsReauthCount = displayedStatuses.filter((status) => status === 'needs_reauth').length
    const syncingCount = displayedStatuses.filter((status) => status === 'syncing').length
    return [
      poolCardMetric(items.length, t('accountPool.upstreamAccounts.metrics.total'), 'database-outline', 'text-primary'),
      poolCardMetric(oauthCount, t('accountPool.upstreamAccounts.metrics.oauth'), 'badge-account-horizontal-outline', 'text-success'),
      poolCardMetric(apiKeyCount, t('accountPool.upstreamAccounts.metrics.apiKey'), 'key-outline', 'text-info'),
      poolCardMetric(
        needsReauthCount + syncingCount,
        t('accountPool.upstreamAccounts.metrics.attention'),
        'alert-decagram-outline',
        'text-warning',
      ),
    ]
  }, [items, t])

  const availableGroups = useMemo(() => {
    let hasUngrouped = false
    for (const item of items) {
      if (!normalizeGroupName(item.groupName)) {
        hasUngrouped = true
      }
    }
    return {
      names: buildGroupNameSuggestions(items.map((item) => item.groupName), groups, groupDraftNotes),
      hasUngrouped,
    }
  }, [groupDraftNotes, groups, items])

  const resolveGroupNoteForName = (groupName: string) => resolveGroupNote(groups, groupDraftNotes, groupName)
  const resolvePendingGroupNoteForName = (groupName: string) => {
    const normalized = normalizeGroupName(groupName)
    if (!normalized || isExistingGroup(groups, normalized)) return ''
    return groupDraftNotes[normalized]?.trim() ?? ''
  }
  const hasGroupNote = (groupName: string) => resolveGroupNoteForName(groupName).trim().length > 0

  const openGroupNoteEditor = (groupName: string) => {
    if (!writesEnabled) return
    const normalized = normalizeGroupName(groupName)
    if (!normalized) return
    setGroupNoteError(null)
    setGroupNoteEditor({
      open: true,
      groupName: normalized,
      note: resolveGroupNoteForName(normalized),
      existing: isExistingGroup(groups, normalized),
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
      setGroupNoteEditor((current) => ({ ...current, open: false }))
      return
    }

    setGroupNoteBusy(true)
    try {
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
      })
      setGroupDraftNotes((current) => {
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

  const filteredItems = useMemo(() => {
    const normalizedQuery = groupFilterQuery.trim().toLocaleLowerCase()
    const allLabel = t('accountPool.upstreamAccounts.groupFilter.all').toLocaleLowerCase()
    const ungroupedLabel = t('accountPool.upstreamAccounts.groupFilter.ungrouped').toLocaleLowerCase()

    if (!normalizedQuery || normalizedQuery === allLabel) return items
    if (normalizedQuery === ungroupedLabel) {
      return items.filter((item) => !item.groupName?.trim())
    }
    return items.filter((item) =>
      item.groupName?.trim().toLocaleLowerCase().includes(normalizedQuery),
    )
  }, [groupFilterQuery, items, t])

  useEffect(() => {
    if (filteredItems.length === 0) return
    if (filteredItems.some((item) => item.id === selectedId)) return
    selectAccount(filteredItems[0].id)
  }, [filteredItems, selectAccount, selectedId])

  const {
    stats: stickyConversationStats,
    isLoading: stickyConversationLoading,
    error: stickyConversationError,
  } = useUpstreamStickyConversations(selectedId, stickyConversationLimit, Boolean(selectedId && isDetailDrawerOpen))

  const selected = detail ?? selectedSummary
  const selectedRecoveryHint = resolveOauthRecoveryHint(
    detail?.kind ?? selected?.kind ?? '',
    detail?.status ?? selected?.status ?? '',
    detail?.lastError ?? selected?.lastError,
  )
  const selectedVisible = filteredItems.some((item) => item.id === selectedId)
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
  const accountStatusLabel = (status: string) => t(`accountPool.upstreamAccounts.status.${status}`)
  const accountSummaryStatusLabel = (item: UpstreamAccountSummary) =>
    accountStatusLabel(resolveDisplayedStatus(item.status, item.lastError))
  const accountKindLabel = (kind: string) =>
    kind === 'oauth_codex'
      ? t('accountPool.upstreamAccounts.kind.oauth')
      : t('accountPool.upstreamAccounts.kind.apiKey')
  const detailDisplayNameConflict = useMemo(
    () => findDisplayNameConflict(items, draft.displayName, detail?.id ?? null),
    [detail?.id, draft.displayName, items],
  )
  const tagFieldLabels = {
    label: t('accountPool.tags.field.label'),
    add: t('accountPool.tags.field.add'),
    empty: t('accountPool.tags.field.empty'),
    searchPlaceholder: t('accountPool.tags.field.searchPlaceholder'),
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
    setDetailActionError(null)
    setBusyAction('save')
    try {
      const response = await saveAccount(source.id, {
        displayName: draft.displayName.trim() || undefined,
        groupName: draft.groupName.trim(),
        isMother: draft.isMother,
        note: draft.note.trim() || undefined,
        tagIds: draft.tagIds,
        groupNote: resolvePendingGroupNoteForName(draft.groupName) || undefined,
        upstreamBaseUrl:
          source.kind === 'api_key_codex' ? draft.upstreamBaseUrl.trim() || null : undefined,
        apiKey: source.kind === 'api_key_codex' && draft.apiKey.trim() ? draft.apiKey.trim() : undefined,
        localPrimaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localPrimaryLimit) : undefined,
        localSecondaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localSecondaryLimit) : undefined,
        localLimitUnit: source.kind === 'api_key_codex' ? draft.localLimitUnit.trim() || undefined : undefined,
      })
      notifyMotherChange(response)
      setDraft((current) => ({ ...current, apiKey: '' }))
    } catch (err) {
      setDetailActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleSync = async (source: UpstreamAccountSummary) => {
    setDetailActionError(null)
    setBusyAction('sync')
    try {
      await runSync(source.id)
    } catch (err) {
      setDetailActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleToggleEnabled = async (source: UpstreamAccountSummary, enabled: boolean) => {
    setDetailActionError(null)
    setBusyAction('toggle')
    try {
      await saveAccount(source.id, { enabled })
    } catch (err) {
      setDetailActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }


  const handleSaveRouting = async () => {
    setPageActionError(null)
    setBusyAction('routing')
    try {
      await saveRouting({ apiKey: routingDraft.apiKey.trim() })
      setRoutingDraft((current) => ({ ...current, apiKey: '' }))
      setIsRoutingDialogOpen(false)
    } catch (err) {
      setPageActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleDelete = async (source: UpstreamAccountSummary) => {
    setDetailActionError(null)
    setIsDeleteConfirmOpen(false)
    setBusyAction('delete')
    try {
      await removeAccount(source.id)
      setIsDetailDrawerOpen(false)
    } catch (err) {
      setDetailActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

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
                <Button type="button" variant="secondary" onClick={() => void refresh()} disabled={busyAction != null}>
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

            {pageActionError ? (
              <Alert variant="error">
                <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>{pageActionError}</div>
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
                    onClick={() => setIsRoutingDialogOpen(true)}
                    disabled={!writesEnabled}
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
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="section-heading">
                <h2 className="section-title">{t('accountPool.upstreamAccounts.listTitle')}</h2>
                <p className="section-description">{t('accountPool.upstreamAccounts.listDescription')}</p>
              </div>
              <div className="flex flex-wrap items-end gap-3">
                <label className="field min-w-[15rem]">
                  <span className="field-label">{t('accountPool.upstreamAccounts.groupFilterLabel')}</span>
                  <UpstreamAccountGroupCombobox
                    value={groupFilterQuery}
                    suggestions={groupFilterSuggestions}
                    placeholder={t('accountPool.upstreamAccounts.groupFilterPlaceholder')}
                    searchPlaceholder={t('accountPool.upstreamAccounts.groupFilterSearchPlaceholder')}
                    emptyLabel={t('accountPool.upstreamAccounts.groupFilterEmpty')}
                    createLabel={(value) => t('accountPool.upstreamAccounts.groupFilterUseValue', { value })}
                    onValueChange={setGroupFilterQuery}
                  />
                </label>
                {selected && selectedVisible && !isDetailDrawerOpen ? (
                  <Button type="button" variant="outline" onClick={() => setIsDetailDrawerOpen(true)}>
                    <AppIcon name="account-details-outline" className="mr-2 h-4 w-4" aria-hidden />
                    {t('accountPool.upstreamAccounts.actions.openDetails')}
                  </Button>
                ) : null}
                {isLoading ? <Spinner className="text-primary" /> : null}
              </div>
            </div>
            <UpstreamAccountsTable
              items={filteredItems}
              selectedId={selectedId}
              onSelect={handleSelectAccount}
              emptyTitle={t('accountPool.upstreamAccounts.emptyTitle')}
              emptyDescription={t('accountPool.upstreamAccounts.emptyDescription')}
              labels={{
                sync: t('accountPool.upstreamAccounts.table.lastSync'),
                never: t('accountPool.upstreamAccounts.never'),
                group: t('accountPool.upstreamAccounts.fields.groupName'),
                primary: t('accountPool.upstreamAccounts.primaryWindowLabel'),
                secondary: t('accountPool.upstreamAccounts.secondaryWindowLabel'),
                nextReset: t('accountPool.upstreamAccounts.table.nextReset'),
                oauth: t('accountPool.upstreamAccounts.kind.oauth'),
                apiKey: t('accountPool.upstreamAccounts.kind.apiKey'),
                mother: t('accountPool.upstreamAccounts.mother.badge'),
                duplicate: t('accountPool.upstreamAccounts.duplicate.badge'),
                status: accountSummaryStatusLabel,
                statusValue: (item) => resolveDisplayedStatus(item.status, item.lastError),
              }}
            />
          </div>
        </div>
      </section>

      <RoutingSettingsDialog
        open={isRoutingDialogOpen}
        title={t('accountPool.upstreamAccounts.routing.dialogTitle')}
        description={t('accountPool.upstreamAccounts.routing.dialogDescription')}
        closeLabel={t('accountPool.upstreamAccounts.routing.close')}
        apiKeyLabel={t('accountPool.upstreamAccounts.routing.apiKeyLabel')}
        generateLabel={t('accountPool.upstreamAccounts.routing.generate')}
        apiKeyPlaceholder={t('accountPool.upstreamAccounts.routing.apiKeyPlaceholder')}
        cancelLabel={t('accountPool.upstreamAccounts.actions.cancel')}
        saveLabel={t('accountPool.upstreamAccounts.routing.save')}
        apiKey={routingDraft.apiKey}
        busy={busyAction === 'routing'}
        writesEnabled={writesEnabled}
        onApiKeyChange={(value) => setRoutingDraft((current) => ({ ...current, apiKey: value }))}
        onGenerate={() => setRoutingDraft((current) => ({ ...current, apiKey: generatePoolRoutingKey() }))}
        onClose={() => {
          setRoutingDraft(buildRoutingDraft(routing?.maskedApiKey))
          setIsRoutingDialogOpen(false)
        }}
        onSave={() => void handleSaveRouting()}
      />

      <AccountDetailDrawer
        open={Boolean(selected && isDetailDrawerOpen)}
        title={selected?.displayName ?? t('accountPool.upstreamAccounts.detailTitle')}
        subtitle={t('accountPool.upstreamAccounts.detailTitle')}
        closeLabel={t('accountPool.upstreamAccounts.actions.closeDetails')}
        closeDisabled={busyAction != null}
        autoFocusCloseButton={!isDeleteConfirmOpen}
        onPortalContainerChange={setDetailDrawerPortalContainer}
        onClose={handleCloseDetailDrawer}
      >
        {!selected ? (
          <div className="flex min-h-[20rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 text-center">
            <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
              <AppIcon name="account-details-outline" className="h-7 w-7" aria-hidden />
            </div>
            <h3 className="text-lg font-semibold">{t('accountPool.upstreamAccounts.detailEmptyTitle')}</h3>
            <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
              {t('accountPool.upstreamAccounts.detailEmptyDescription')}
            </p>
          </div>
        ) : isDetailLoading && !detail ? (
          <AccountDetailSkeleton />
        ) : (
          <>
            <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
              <div className="space-y-3">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge
                    variant={statusVariant(
                      resolveDisplayedStatus(selected.status, detail?.lastError ?? selected.lastError),
                    )}
                  >
                    {accountStatusLabel(
                      resolveDisplayedStatus(selected.status, detail?.lastError ?? selected.lastError),
                    )}
                  </Badge>
                  <Badge variant={kindVariant(selected.kind)}>{accountKindLabel(selected.kind)}</Badge>
                  {selected.planType ? <Badge variant="secondary">{selected.planType}</Badge> : null}
                  {selected.duplicateInfo ? (
                    <Badge variant="warning">
                      {t('accountPool.upstreamAccounts.duplicate.badge')}
                    </Badge>
                  ) : null}
                  {selected.kind === 'api_key_codex' ? (
                    <Badge variant="secondary">{t('accountPool.upstreamAccounts.apiKey.localPlaceholder')}</Badge>
                  ) : null}
                </div>
                <div className="section-heading">
                  <div className="flex flex-wrap items-center gap-2">
                    <h3 className="section-title">{selected.displayName}</h3>
                    {selected.isMother ? <MotherAccountBadge label={t('accountPool.upstreamAccounts.mother.badge')} /> : null}
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
                    disabled={busyAction === 'toggle' || !writesEnabled}
                    aria-label={t('accountPool.upstreamAccounts.actions.enable')}
                  />
                </div>
                <Button type="button" variant="secondary" onClick={() => void handleSync(selected)} disabled={busyAction === 'sync'}>
                  {busyAction === 'sync' ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="refresh-circle" className="mr-2 h-4 w-4" aria-hidden />}
                  {t('accountPool.upstreamAccounts.actions.syncNow')}
                </Button>
                {selected.kind === 'oauth_codex' ? (
                  <Button type="button" variant="outline" onClick={() => void handleOauthLogin(selected.id)} disabled={busyAction === 'relogin' || !writesEnabled}>
                    {busyAction === 'relogin' ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="login-variant" className="mr-2 h-4 w-4" aria-hidden />}
                    {t('accountPool.upstreamAccounts.actions.relogin')}
                  </Button>
                ) : null}
                <Popover
                  open={isDeleteConfirmOpen}
                  onOpenChange={(nextOpen) => {
                    if (busyAction === 'delete' && !nextOpen) return
                    if (nextOpen) {
                      setDetailActionError(null)
                    }
                    setIsDeleteConfirmOpen(nextOpen)
                  }}
                >
                  <PopoverTrigger asChild>
                    <Button
                      type="button"
                      variant="destructive"
                      disabled={busyAction === 'delete' || !writesEnabled}
                      aria-haspopup="dialog"
                      aria-expanded={isDeleteConfirmOpen}
                      aria-controls={isDeleteConfirmOpen ? deleteConfirmTitleId : undefined}
                    >
                      {busyAction === 'delete' ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="trash-can-outline" className="mr-2 h-4 w-4" aria-hidden />}
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
                            disabled={busyAction === 'delete' || !writesEnabled}
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

            <div className="grid gap-5">
              {detailActionError ? (
                <Alert variant="error">
                  <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  <div>{detailActionError}</div>
                </Alert>
              ) : null}
              {detail ? (
                <>
                {detail.duplicateInfo ? (
                  <Alert variant="warning">
                    <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                    <div>
                      <p className="font-medium">
                        {t('accountPool.upstreamAccounts.duplicate.badge')}
                      </p>
                      <p className="mt-1 text-sm text-warning/90">
                        {t('accountPool.upstreamAccounts.duplicate.warningBody', {
                          reasons: formatDuplicateReasons(detail.duplicateInfo),
                          peers: detail.duplicateInfo.peerAccountIds.join(', '),
                        })}
                      </p>
                    </div>
                  </Alert>
                ) : null}
                <div className="metric-grid">
                  <DetailField label={t('accountPool.upstreamAccounts.fields.groupName')} value={detail.groupName ?? ''} />
                  <DetailField
                    label={t('accountPool.upstreamAccounts.mother.fieldLabel')}
                    value={detail.isMother ? t('accountPool.upstreamAccounts.mother.badge') : t('accountPool.upstreamAccounts.mother.notMother')}
                  />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.email')} value={detail.email ?? ''} />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.accountId')} value={detail.chatgptAccountId ?? detail.maskedApiKey ?? ''} />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.userId')} value={detail.chatgptUserId ?? ''} />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.lastSuccessSync')} value={formatDateTime(detail.lastSuccessfulSyncAt)} />
                </div>

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
                          message={t(
                            'accountPool.upstreamAccounts.validation.displayNameDuplicate',
                          )}
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
                        variant={hasGroupNote(draft.groupName) ? 'secondary' : 'outline'}
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
                    {detail.kind === 'api_key_codex' ? (
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
                        onClick={() => void handleSave(detail)}
                        disabled={
                          busyAction === 'save' ||
                          !writesEnabled ||
                          detailDisplayNameConflict != null ||
                          (detail.kind === 'api_key_codex' && Boolean(draftUpstreamBaseUrlError))
                        }
                      >
                        {busyAction === 'save' ? <Spinner size="sm" className="mr-2" /> : <AppIcon name="content-save-outline" className="mr-2 h-4 w-4" aria-hidden />}
                        {t('accountPool.upstreamAccounts.actions.save')}
                      </Button>
                    </div>
                  </CardContent>
                </Card>

                <div className="grid gap-4 xl:grid-cols-2">
                  <UpstreamAccountUsageCard
                    title={t('accountPool.upstreamAccounts.primaryWindowLabel')}
                    description={t('accountPool.upstreamAccounts.usage.primaryDescription')}
                    window={detail.primaryWindow}
                    history={detail.history}
                    historyKey="primaryUsedPercent"
                    emptyLabel={t('accountPool.upstreamAccounts.noHistory')}
                    noteLabel={detail.kind === 'api_key_codex' ? t('accountPool.upstreamAccounts.apiKey.localPlaceholder') : undefined}
                  />
                  <UpstreamAccountUsageCard
                    title={t('accountPool.upstreamAccounts.secondaryWindowLabel')}
                    description={t('accountPool.upstreamAccounts.usage.secondaryDescription')}
                    window={detail.secondaryWindow}
                    history={detail.history}
                    historyKey="secondaryUsedPercent"
                    emptyLabel={t('accountPool.upstreamAccounts.noHistory')}
                    noteLabel={detail.kind === 'api_key_codex' ? t('accountPool.upstreamAccounts.apiKey.localPlaceholder') : undefined}
                    accentClassName="text-secondary"
                  />
                </div>

                <EffectiveRoutingRuleCard
                  rule={detail.effectiveRoutingRule}
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
                    <label className="field w-36">
                      <span className="field-label">{t('accountPool.upstreamAccounts.stickyConversations.limitLabel')}</span>
                      <select
                        name="stickyConversationLimit"
                        className="field-select field-select-sm"
                        value={stickyConversationLimit}
                        onChange={(event) => setStickyConversationLimit(Number(event.target.value))}
                      >
                        {STICKY_CONVERSATION_LIMIT_OPTIONS.map((value) => (
                          <option key={value} value={value}>
                            {t('accountPool.upstreamAccounts.stickyConversations.limitOption', { count: value })}
                          </option>
                        ))}
                      </select>
                    </label>
                  </CardHeader>
                  <CardContent>
                    <StickyKeyConversationTable
                      stats={stickyConversationStats}
                      isLoading={stickyConversationLoading}
                      error={stickyConversationError}
                    />
                  </CardContent>
                </Card>

                <Card className="border-base-300/80 bg-base-100/72">
                  <CardHeader>
                    <CardTitle>{t('accountPool.upstreamAccounts.healthTitle')}</CardTitle>
                    <CardDescription>{t('accountPool.upstreamAccounts.healthDescription')}</CardDescription>
                  </CardHeader>
                  <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                    <DetailField label={t('accountPool.upstreamAccounts.fields.lastSyncedAt')} value={formatDateTime(detail.lastSyncedAt)} />
                    <DetailField label={t('accountPool.upstreamAccounts.fields.lastRefreshedAt')} value={formatDateTime(detail.lastRefreshedAt)} />
                    <DetailField label={t('accountPool.upstreamAccounts.fields.tokenExpiresAt')} value={formatDateTime(detail.tokenExpiresAt)} />
                    <DetailField
                      label={t('accountPool.upstreamAccounts.fields.credits')}
                      value={detail.credits?.balance ? `${detail.credits.balance}` : detail.credits?.unlimited ? t('accountPool.upstreamAccounts.unlimited') : t('accountPool.upstreamAccounts.unavailable')}
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
                      <p className="metric-label">{t('accountPool.upstreamAccounts.fields.lastError')}</p>
                      <p className="mt-2 text-sm leading-6 text-base-content/75">{detail.lastError ?? t('accountPool.upstreamAccounts.noError')}</p>
                      <p className="mt-2 text-xs text-base-content/55">{formatDateTime(detail.lastErrorAt)}</p>
                    </div>
                  </CardContent>
                </Card>
                </>
              ) : null}
            </div>
          </>
        )}
      </AccountDetailDrawer>

      <UpstreamAccountGroupNoteDialog
        open={groupNoteEditor.open}
        groupName={groupNoteEditor.groupName}
        note={groupNoteEditor.note}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null)
          setGroupNoteEditor((current) => ({ ...current, note: value }))
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
      />

      {error ? (
        <Alert variant="warning">
          <AppIcon name="information-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <div>{error}</div>
        </Alert>
      ) : null}
    </div>
  )
}
