import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AppIcon } from '../../components/AppIcon'
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
  DialogHeader,
  DialogTitle,
} from '../../components/ui/dialog'
import { FloatingFieldError } from '../../components/ui/floating-field-error'
import { FormFieldFeedback } from '../../components/ui/form-field-feedback'
import { Input } from '../../components/ui/input'
import {
  Popover,
  PopoverAnchor,
  PopoverArrow,
  PopoverContent,
  PopoverTrigger,
} from '../../components/ui/popover'
import { Spinner } from '../../components/ui/spinner'
import { Tooltip } from '../../components/ui/tooltip'
import { OauthMailboxChip } from '../../components/account-pool/OauthMailboxChip'
import { AccountTagField } from '../../components/AccountTagField'
import { UpstreamAccountGroupCombobox } from '../../components/UpstreamAccountGroupCombobox'
import { UpstreamAccountGroupNoteDialog } from '../../components/UpstreamAccountGroupNoteDialog'
import { MotherAccountToggle } from '../../components/MotherAccountToggle'
import { useMotherSwitchNotifications } from '../../hooks/useMotherSwitchNotifications'
import { usePoolTags } from '../../hooks/usePoolTags'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import type {
  LoginSessionStatusResponse,
  OauthMailboxSession,
  OauthMailboxStatus,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from '../../lib/api'
import { fetchUpstreamAccountDetail } from '../../lib/api'
import { copyText, selectAllReadonlyText } from '../../lib/clipboard'
import { emitUpstreamAccountsChanged } from '../../lib/upstreamAccountsEvents'
import {
  buildGroupNameSuggestions,
  isExistingGroup,
  normalizeGroupName,
  resolveGroupNote,
} from '../../lib/upstreamAccountGroups'
import { validateUpstreamBaseUrl } from '../../lib/upstreamBaseUrl'
import { applyMotherUpdateToItems, normalizeMotherGroupKey } from '../../lib/upstreamMother'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

type CreateTab = 'oauth' | 'batchOauth' | 'apiKey'
type BatchOauthBusyAction = 'generate' | 'complete' | null
type DuplicateWarningState = {
  accountId: number
  displayName: string
  peerAccountIds: number[]
  reasons: string[]
}
type GroupNoteEditorState = {
  open: boolean
  groupName: string
  note: string
  existing: boolean
}
type MailboxCopyTone = 'idle' | 'copied'

type BatchOauthRow = {
  id: string
  displayName: string
  groupName: string
  isMother: boolean
  note: string
  noteExpanded: boolean
  callbackUrl: string
  session: LoginSessionStatusResponse | null
  sessionHint: string | null
  duplicateWarning: DuplicateWarningState | null
  needsRefresh: boolean
  actionError: string | null
  busyAction: BatchOauthBusyAction
  mailboxSession: OauthMailboxSession | null
  mailboxInput: string
  mailboxStatus: OauthMailboxStatus | null
  mailboxTone: MailboxCopyTone
  mailboxCodeTone: MailboxCopyTone
  mailboxBusy: boolean
}

type CreatePageDraft = {
  oauth?: {
    displayName?: string
    groupName?: string
    isMother?: boolean
    note?: string
    tagIds?: number[]
    callbackUrl?: string
    session?: LoginSessionStatusResponse | null
    sessionHint?: string | null
    duplicateWarning?: DuplicateWarningState | null
    actionError?: string | null
    mailboxSession?: OauthMailboxSession | null
    mailboxInput?: string
    mailboxStatus?: OauthMailboxStatus | null
    mailboxTone?: MailboxCopyTone
    mailboxCodeTone?: MailboxCopyTone
  }
  batchOauth?: {
    defaultGroupName?: string
    tagIds?: number[]
    rows?: Array<Partial<BatchOauthRow> & { id?: string }>
  }
  apiKey?: {
    displayName?: string
    groupName?: string
    isMother?: boolean
    note?: string
    tagIds?: number[]
    apiKeyValue?: string
    upstreamBaseUrl?: string
    primaryLimit?: string
    secondaryLimit?: string
    limitUnit?: string
  }
}

type CreatePageLocationState = {
  draft?: CreatePageDraft
} | null

function normalizeNumberInput(value: string): number | undefined {
  const trimmed = value.trim()
  if (!trimmed) return undefined
  const parsed = Number(trimmed)
  return Number.isFinite(parsed) ? parsed : undefined
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

function parseAccountId(search: string): number | null {
  const value = new URLSearchParams(search).get('accountId')
  if (!value) return null
  const parsed = Number(value)
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null
}

function parseCreateMode(search: string): CreateTab {
  const value = new URLSearchParams(search).get('mode')
  if (value === 'batchOauth') return 'batchOauth'
  if (value === 'apiKey') return 'apiKey'
  return 'oauth'
}

function createBatchOauthRow(id: string, groupName = ''): BatchOauthRow {
  return {
    id,
    displayName: '',
    groupName,
    isMother: false,
    note: '',
    noteExpanded: false,
    callbackUrl: '',
    session: null,
    sessionHint: null,
    duplicateWarning: null,
    needsRefresh: false,
    actionError: null,
    busyAction: null,
    mailboxSession: null,
    mailboxInput: '',
    mailboxStatus: null,
    mailboxTone: 'idle',
    mailboxCodeTone: 'idle',
    mailboxBusy: false,
  }
}

function hydrateBatchOauthRow(
  seed: Partial<BatchOauthRow> & { id?: string },
  fallbackId: string,
  fallbackGroupName = '',
): BatchOauthRow {
  return {
    ...createBatchOauthRow(seed.id ?? fallbackId, seed.groupName ?? fallbackGroupName),
    ...seed,
    id: seed.id ?? fallbackId,
    groupName: seed.groupName ?? fallbackGroupName,
    isMother: seed.isMother === true,
    duplicateWarning: seed.duplicateWarning ?? null,
    needsRefresh: seed.needsRefresh === true,
    mailboxSession: seed.mailboxSession ?? null,
    mailboxInput:
      typeof seed.mailboxInput === 'string'
        ? seed.mailboxInput
        : seed.mailboxSession?.emailAddress ?? '',
    mailboxStatus: seed.mailboxStatus ?? null,
    mailboxTone: seed.mailboxTone === 'copied' ? 'copied' : 'idle',
    mailboxCodeTone: seed.mailboxCodeTone === 'copied' ? 'copied' : 'idle',
    mailboxBusy: seed.mailboxBusy === true,
  }
}

function getNextBatchRowIndex(rows: BatchOauthRow[]) {
  return rows.reduce((max, row) => {
    const matched = /^row-(\d+)$/.exec(row.id)
    const current = matched ? Number(matched[1]) : 0
    return Number.isFinite(current) ? Math.max(max, current + 1) : max
  }, 1)
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

function invalidatePendingSingleOauthSession(
  currentSession: LoginSessionStatusResponse | null,
  setSession: (value: LoginSessionStatusResponse | null) => void,
  setSessionHint: (value: string | null) => void,
  setOauthCallbackUrl: (value: string) => void,
  setManualCopyOpen: (value: boolean) => void,
  setActionError: (value: string | null) => void,
  setOauthDuplicateWarning: (value: DuplicateWarningState | null) => void,
  regenerateRequiredLabel: string,
) {
  if (!currentSession) return
  setSession(null)
  setSessionHint(regenerateRequiredLabel)
  setOauthCallbackUrl('')
  setManualCopyOpen(false)
  setActionError(null)
  setOauthDuplicateWarning(null)
}

function applyBatchMotherDraftRules(rows: BatchOauthRow[], changedRowId: string) {
  const changedRow = rows.find((row) => row.id === changedRowId)
  if (!changedRow?.isMother) return rows
  const groupKey = normalizeMotherGroupKey(changedRow.groupName)
  return rows.map((row) =>
    row.id !== changedRowId && row.isMother && normalizeMotherGroupKey(row.groupName) === groupKey
      ? { ...row, isMother: false }
      : row,
  )
}

function enforceBatchMotherDraftUniqueness(rows: BatchOauthRow[]) {
  const winners = new Map<string, string>()
  for (const row of rows) {
    if (!row.isMother) continue
    winners.set(normalizeMotherGroupKey(row.groupName), row.id)
  }
  return rows.map((row) =>
    row.isMother && winners.get(normalizeMotherGroupKey(row.groupName)) !== row.id
      ? { ...row, isMother: false }
      : row,
  )
}

function batchStatusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'completed') return 'success'
  if (status === 'completedNeedsRefresh') return 'warning'
  if (status === 'pending') return 'warning'
  if (status === 'failed' || status === 'expired') return 'error'
  return 'secondary'
}

function batchRowStatus(row: BatchOauthRow) {
  if (row.needsRefresh) return 'completedNeedsRefresh'
  return row.session?.status ?? 'draft'
}

function batchRowStatusDetail(row: BatchOauthRow) {
  if (row.actionError) return row.actionError
  if (row.sessionHint) return row.sessionHint
  if (row.session?.error) return row.session.error
  if (row.session?.expiresAt) return formatDateTime(row.session.expiresAt)
  return null
}

function batchMailboxCodeVariant(row: BatchOauthRow): 'default' | 'secondary' | 'outline' {
  const code = row.mailboxStatus?.latestCode?.value
  if (!code) return 'secondary'
  return row.mailboxCodeTone === 'copied' ? 'outline' : 'default'
}

function buildActionTooltip(title: string, description: string) {
  return (
    <div className="space-y-1">
      <p className="font-semibold text-base-content">{title}</p>
      <p className="leading-5 text-base-content/70">{description}</p>
    </div>
  )
}

function DuplicateWarningPopover({
  duplicateWarning,
  summaryTitle,
  summaryBody,
  openDetailsLabel,
  onOpenDetails,
  side = 'top',
}: {
  duplicateWarning: DuplicateWarningState
  summaryTitle: string
  summaryBody: string
  openDetailsLabel: string
  onOpenDetails: (accountId: number) => void
  side?: 'top' | 'right' | 'bottom' | 'left'
}) {
  const [open, setOpen] = useState(false)

  useEffect(() => {
    setOpen(true)
  }, [duplicateWarning.accountId, summaryTitle, summaryBody])

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex h-5 w-5 shrink-0 items-center justify-center text-warning transition-colors hover:text-warning/90"
          aria-label={summaryTitle}
        >
          <AppIcon name="alert-outline" className="h-5 w-5" aria-hidden />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        side={side}
        sideOffset={10}
        onOpenAutoFocus={(event) => event.preventDefault()}
        className="w-[16.5rem] rounded-2xl border border-warning/45 bg-base-100 p-0 shadow-[0_16px_38px_rgba(15,23,42,0.16)]"
      >
        <div className="space-y-3 p-3">
          <div className="flex items-start gap-3">
            <div className="mt-0.5 text-warning">
              <AppIcon name="alert-outline" className="h-4 w-4" aria-hidden />
            </div>
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-semibold leading-5 text-warning">{summaryTitle}</p>
              <p className="text-[11px] leading-5 text-base-content/72">{summaryBody}</p>
            </div>
          </div>
          <div className="flex justify-end">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              className="h-7 rounded-full px-2.5 text-xs font-semibold text-warning hover:bg-warning/10 hover:text-warning"
              onClick={() => {
                setOpen(false)
                onOpenDetails(duplicateWarning.accountId)
              }}
            >
              {openDetailsLabel}
            </Button>
          </div>
        </div>
        <PopoverArrow className="fill-base-100 stroke-warning/45 stroke-[0.8]" width={16} height={8} />
      </PopoverContent>
    </Popover>
  )
}

function DuplicateDetailField({
  label,
  value,
}: {
  label: string
  value?: string | null
}) {
  return (
    <div className="rounded-2xl border border-base-300/70 bg-base-100/82 px-3 py-3">
      <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/45">
        {label}
      </p>
      <p className="mt-1 break-all text-sm text-base-content/82">{value?.trim() ? value : '—'}</p>
    </div>
  )
}

function accountStatusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'active') return 'success'
  if (status === 'syncing') return 'warning'
  if (status === 'error' || status === 'needs_reauth') return 'error'
  return 'secondary'
}

function accountKindVariant(kind: string): 'secondary' | 'success' {
  return kind === 'oauth_codex' ? 'success' : 'secondary'
}

function DuplicateAccountDetailDialog({
  open,
  detail,
  isLoading,
  onClose,
  title,
  description,
  duplicateLabel,
  closeLabel,
  formatDuplicateReasons,
  statusLabel,
  kindLabel,
  fieldLabels,
}: {
  open: boolean
  detail: UpstreamAccountDetail | null
  isLoading: boolean
  onClose: () => void
  title: string
  description: string
  duplicateLabel: string
  closeLabel: string
  formatDuplicateReasons: (duplicateInfo?: UpstreamAccountDuplicateInfo | null) => string
  statusLabel: (status: string) => string
  kindLabel: (kind: string) => string
  fieldLabels: {
    groupName: string
    email: string
    accountId: string
    userId: string
    lastSuccessSync: string
  }
}) {
  return (
    <Dialog open={open} onOpenChange={(nextOpen) => !nextOpen && onClose()}>
      <DialogContent className="max-h-[85vh] overflow-hidden p-0 sm:max-w-[38rem]">
        <div className="flex items-start justify-between gap-4 border-b border-base-300/70 px-5 py-4">
          <DialogHeader className="min-w-0">
            <DialogTitle className="truncate">{detail?.displayName ?? title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} />
        </div>
        <div className="space-y-4 overflow-y-auto px-5 py-5">
          {isLoading ? (
            <div className="flex min-h-44 items-center justify-center">
              <Spinner />
            </div>
          ) : detail ? (
            <>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={accountStatusVariant(detail.status)}>{statusLabel(detail.status)}</Badge>
                <Badge variant={accountKindVariant(detail.kind)}>{kindLabel(detail.kind)}</Badge>
                {detail.duplicateInfo ? <Badge variant="warning">{duplicateLabel}</Badge> : null}
              </div>
              {detail.duplicateInfo ? (
                <Alert variant="warning">
                  <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  <div>
                    <p className="font-semibold text-warning">{duplicateLabel}</p>
                    <p className="mt-1 text-sm text-warning/90">
                      {`命中：${formatDuplicateReasons(detail.duplicateInfo)}。关联账号 ID：${detail.duplicateInfo.peerAccountIds.join(', ') || '—'}。`}
                    </p>
                  </div>
                </Alert>
              ) : null}
              <div className="grid gap-3 md:grid-cols-2">
                <DuplicateDetailField label={fieldLabels.groupName} value={detail.groupName ?? ''} />
                <DuplicateDetailField label={fieldLabels.email} value={detail.email ?? ''} />
                <DuplicateDetailField
                  label={fieldLabels.accountId}
                  value={detail.chatgptAccountId ?? detail.maskedApiKey ?? ''}
                />
                <DuplicateDetailField label={fieldLabels.userId} value={detail.chatgptUserId ?? ''} />
                <DuplicateDetailField
                  label={fieldLabels.lastSuccessSync}
                  value={formatDateTime(detail.lastSuccessfulSyncAt)}
                />
              </div>
            </>
          ) : (
            <p className="text-sm text-base-content/65">{description}</p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

export default function UpstreamAccountCreatePage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const location = useLocation()
  const locationState = (location.state as CreatePageLocationState) ?? null
  const draft = locationState?.draft ?? null
  const {
    items,
    groups = [],
    writesEnabled,
    isLoading,
    error,
    beginOauthLogin,
    beginOauthMailboxSession,
    getLoginSession,
    getOauthMailboxStatuses,
    removeOauthMailboxSession,
    completeOauthLogin,
    createApiKeyAccount,
    saveGroupNote,
  } = useUpstreamAccounts()
  const { items: tagItems, createTag, updateTag, deleteTag } = usePoolTags()
  const notifyMotherSwitches = useMotherSwitchNotifications()

  const relinkAccountId = useMemo(() => parseAccountId(location.search), [location.search])
  const relinkSummary = useMemo(
    () => (relinkAccountId == null ? null : items.find((item) => item.id === relinkAccountId) ?? null),
    [items, relinkAccountId],
  )
  const isRelinking = relinkAccountId != null
  const initialBatchRows = useMemo(() => {
    const defaultGroupName = draft?.batchOauth?.defaultGroupName ?? ''
    if (!draft?.batchOauth?.rows?.length) {
      return Array.from({ length: 5 }, (_, index) => createBatchOauthRow(`row-${index + 1}`, defaultGroupName))
    }
    return draft.batchOauth.rows.map((row, index) =>
      hydrateBatchOauthRow(row, `row-${index + 1}`, defaultGroupName),
    )
  }, [draft])

  const [activeTab, setActiveTab] = useState<CreateTab>(() => (isRelinking ? 'oauth' : parseCreateMode(location.search)))
  const [oauthDisplayName, setOauthDisplayName] = useState(() => draft?.oauth?.displayName ?? '')
  const [oauthGroupName, setOauthGroupName] = useState(() => draft?.oauth?.groupName ?? '')
  const [oauthIsMother, setOauthIsMother] = useState(() => draft?.oauth?.isMother === true)
  const [oauthNote, setOauthNote] = useState(() => draft?.oauth?.note ?? '')
  const [oauthTagIds, setOauthTagIds] = useState<number[]>(() => draft?.oauth?.tagIds ?? [])
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState(() => draft?.oauth?.callbackUrl ?? '')
  const [oauthMailboxSession, setOauthMailboxSession] = useState<OauthMailboxSession | null>(
    () => draft?.oauth?.mailboxSession ?? null,
  )
  const [oauthMailboxInput, setOauthMailboxInput] = useState(
    () => draft?.oauth?.mailboxInput ?? draft?.oauth?.mailboxSession?.emailAddress ?? '',
  )
  const [oauthMailboxStatus, setOauthMailboxStatus] = useState<OauthMailboxStatus | null>(
    () => draft?.oauth?.mailboxStatus ?? null,
  )
  const [oauthMailboxTone, setOauthMailboxTone] = useState<MailboxCopyTone>(
    () => draft?.oauth?.mailboxTone ?? 'idle',
  )
  const [oauthMailboxCodeTone, setOauthMailboxCodeTone] = useState<MailboxCopyTone>(
    () => draft?.oauth?.mailboxCodeTone ?? 'idle',
  )
  const [oauthMailboxBusy, setOauthMailboxBusy] = useState(false)
  const [apiKeyDisplayName, setApiKeyDisplayName] = useState(() => draft?.apiKey?.displayName ?? '')
  const [apiKeyGroupName, setApiKeyGroupName] = useState(() => draft?.apiKey?.groupName ?? '')
  const [apiKeyIsMother, setApiKeyIsMother] = useState(() => draft?.apiKey?.isMother === true)
  const [apiKeyNote, setApiKeyNote] = useState(() => draft?.apiKey?.note ?? '')
  const [apiKeyTagIds, setApiKeyTagIds] = useState<number[]>(() => draft?.apiKey?.tagIds ?? [])
  const [apiKeyValue, setApiKeyValue] = useState(() => draft?.apiKey?.apiKeyValue ?? '')
  const [apiKeyUpstreamBaseUrl, setApiKeyUpstreamBaseUrl] = useState(() => draft?.apiKey?.upstreamBaseUrl ?? '')
  const [apiKeyPrimaryLimit, setApiKeyPrimaryLimit] = useState(() => draft?.apiKey?.primaryLimit ?? '')
  const [apiKeySecondaryLimit, setApiKeySecondaryLimit] = useState(() => draft?.apiKey?.secondaryLimit ?? '')
  const [apiKeyLimitUnit, setApiKeyLimitUnit] = useState(() => draft?.apiKey?.limitUnit ?? 'requests')
  const [session, setSession] = useState<LoginSessionStatusResponse | null>(() => draft?.oauth?.session ?? null)
  const [sessionHint, setSessionHint] = useState<string | null>(() => draft?.oauth?.sessionHint ?? null)
  const [oauthDuplicateWarning, setOauthDuplicateWarning] = useState<DuplicateWarningState | null>(
    () => draft?.oauth?.duplicateWarning ?? null,
  )
  const [duplicateDetailOpen, setDuplicateDetailOpen] = useState(false)
  const [duplicateDetailLoading, setDuplicateDetailLoading] = useState(false)
  const [duplicateDetail, setDuplicateDetail] = useState<UpstreamAccountDetail | null>(null)
  const [actionError, setActionError] = useState<string | null>(() => draft?.oauth?.actionError ?? null)
  const [busyAction, setBusyAction] = useState<string | null>(null)
  const [manualCopyOpen, setManualCopyOpen] = useState(false)
  const [batchManualCopyRowId, setBatchManualCopyRowId] = useState<string | null>(null)
  const [batchDefaultGroupName, setBatchDefaultGroupName] = useState(() => draft?.batchOauth?.defaultGroupName ?? '')
  const [batchTagIds, setBatchTagIds] = useState<number[]>(() => draft?.batchOauth?.tagIds ?? [])
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([])
  const [batchRows, setBatchRows] = useState<BatchOauthRow[]>(() => initialBatchRows)
  const [groupDraftNotes, setGroupDraftNotes] = useState<Record<string, string>>({})
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupNoteEditorState>({
    open: false,
    groupName: '',
    note: '',
    existing: false,
  })
  const [groupNoteBusy, setGroupNoteBusy] = useState(false)
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null)
  const oauthMailboxToneResetRef = useRef<number | null>(null)
  const batchMailboxToneResetRef = useRef<Record<string, number>>({})

  useEffect(() => {
    return () => {
      if (oauthMailboxToneResetRef.current != null) {
        window.clearTimeout(oauthMailboxToneResetRef.current)
      }
      Object.values(batchMailboxToneResetRef.current).forEach((timerId) => {
        window.clearTimeout(timerId)
      })
    }
  }, [])
  const batchRowIdRef = useRef(getNextBatchRowIndex(initialBatchRows))
  const manualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null)
  const batchManualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null)

  const groupSuggestions = useMemo(
    () => buildGroupNameSuggestions(items.map((item) => item.groupName), groups, groupDraftNotes),
    [groupDraftNotes, groups, items],
  )
  const oauthConflictExcludeId =
    relinkAccountId ??
    (session?.status === 'completed' ? (session.accountId ?? null) : null)
  const oauthDisplayNameConflict = useMemo(
    () => findDisplayNameConflict(items, oauthDisplayName, oauthConflictExcludeId),
    [items, oauthConflictExcludeId, oauthDisplayName],
  )
  const apiKeyDisplayNameConflict = useMemo(
    () => findDisplayNameConflict(items, apiKeyDisplayName),
    [apiKeyDisplayName, items],
  )
  const batchDraftNameCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const row of batchRows) {
      if (row.session?.status === 'completed') continue
      const key = normalizeDisplayNameKey(row.displayName)
      if (!key) continue
      counts.set(key, (counts.get(key) ?? 0) + 1)
    }
    return counts
  }, [batchRows])
  const batchDisplayNameError = (row: BatchOauthRow) => {
    const existingConflict = findDisplayNameConflict(items, row.displayName, row.session?.accountId ?? null)
    if (existingConflict) {
      return t('accountPool.upstreamAccounts.validation.displayNameDuplicate')
    }
    const key = normalizeDisplayNameKey(row.displayName)
    if (key && (batchDraftNameCounts.get(key) ?? 0) > 1) {
      return t('accountPool.upstreamAccounts.validation.displayNameDuplicate')
    }
    return null
  }
  const formatDuplicateReasons = (
    duplicateInfo?: UpstreamAccountDuplicateInfo | null,
  ) => {
    const reasons = duplicateInfo?.reasons ?? []
    return reasons
      .map((reason) => {
        if (reason === 'sharedChatgptAccountId') {
          return t('accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId')
        }
        if (reason === 'sharedChatgptUserId') {
          return t('accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId')
        }
        return reason
      })
      .join(' / ')
  }
  const accountStatusLabel = (status: string) => t(`accountPool.upstreamAccounts.status.${status}`)
  const accountKindLabel = (kind: string) =>
    kind === 'oauth_codex'
      ? t('accountPool.upstreamAccounts.kind.oauth')
      : t('accountPool.upstreamAccounts.kind.apiKey')
  const openDuplicateDetailDialog = async (accountId: number) => {
    setDuplicateDetailOpen(true)
    setDuplicateDetailLoading(true)
    try {
      const response = await fetchUpstreamAccountDetail(accountId)
      setDuplicateDetail(response)
    } catch {
      setDuplicateDetail(null)
    } finally {
      setDuplicateDetailLoading(false)
    }
  }
  const apiKeyUpstreamBaseUrlError = useMemo(() => {
    const code = validateUpstreamBaseUrl(apiKeyUpstreamBaseUrl)
    if (code === 'invalid_absolute_url') {
      return t('accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid')
    }
    if (code === 'query_or_fragment_not_allowed') {
      return t('accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment')
    }
    return null
  }, [apiKeyUpstreamBaseUrl, t])
  const oauthMailboxAddress = oauthMailboxSession?.emailAddress ?? oauthMailboxInput

  const handleCreateTag = async (payload: Parameters<typeof createTag>[0]) => {
    const detail = await createTag(payload)
    setPageCreatedTagIds((current) => (current.includes(detail.id) ? current : [...current, detail.id]))
    return detail
  }

  const handleDeleteTag = async (tagId: number) => {
    await deleteTag(tagId)
    setPageCreatedTagIds((current) => current.filter((value) => value !== tagId))
    setOauthTagIds((current) => current.filter((value) => value !== tagId))
    setApiKeyTagIds((current) => current.filter((value) => value !== tagId))
    setBatchTagIds((current) => current.filter((value) => value !== tagId))
  }

  useEffect(() => {
    if (isRelinking) {
      setActiveTab('oauth')
      return
    }
    setActiveTab(parseCreateMode(location.search))
  }, [isRelinking, location.search])

  useEffect(() => {
    if (!isRelinking || !relinkSummary) return
    setActiveTab('oauth')
    setOauthDisplayName((current) => current || relinkSummary.displayName)
    setOauthGroupName((current) => current || relinkSummary.groupName || '')
    setOauthTagIds((current) => (current.length > 0 ? current : (relinkSummary.tags ?? []).map((tag) => tag.id)))
    setOauthIsMother((current) => current || relinkSummary.isMother)
  }, [isRelinking, relinkSummary])

  useEffect(() => {
    if (!manualCopyOpen) return
    const frame = window.requestAnimationFrame(() => {
      selectAllReadonlyText(manualCopyFieldRef.current)
    })
    return () => window.cancelAnimationFrame(frame)
  }, [manualCopyOpen])

  useEffect(() => {
    if (!batchManualCopyRowId) return
    const frame = window.requestAnimationFrame(() => {
      selectAllReadonlyText(batchManualCopyFieldRef.current)
    })
    return () => window.cancelAnimationFrame(frame)
  }, [batchManualCopyRowId])

  useEffect(() => {
    if (!oauthMailboxSession) return
    let cancelled = false
    const poll = async () => {
      try {
        const [status] = await getOauthMailboxStatuses([oauthMailboxSession.sessionId])
        if (cancelled || !status) return
        setOauthMailboxStatus(status)
        if (status.latestCode?.value && status.latestCode.value !== oauthMailboxStatus?.latestCode?.value) {
          setOauthMailboxCodeTone('idle')
        }
      } catch {
        // Ignore transient mailbox polling errors and keep the page interactive.
      }
    }
    void poll()
    const timer = window.setInterval(() => {
      void poll()
    }, 5000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [
    getOauthMailboxStatuses,
    oauthMailboxSession,
    oauthMailboxStatus?.latestCode?.value,
  ])

  useEffect(() => {
    const activeRows = batchRows.filter((row) => row.mailboxSession)
    if (activeRows.length === 0) return
    let cancelled = false
    const previousCodeMap = new Map(
      activeRows.map((row) => [row.id, row.mailboxStatus?.latestCode?.value ?? null]),
    )
    const poll = async () => {
      try {
        const statuses = await getOauthMailboxStatuses(
          activeRows
            .map((row) => row.mailboxSession?.sessionId ?? '')
            .filter((value) => value.length > 0),
        )
        if (cancelled) return
        const bySessionId = new Map(statuses.map((status) => [status.sessionId, status]))
        setBatchRows((current) =>
          current.map((row) => {
            const sessionId = row.mailboxSession?.sessionId
            if (!sessionId) {
              return row
            }
            const nextStatus = bySessionId.get(sessionId) ?? row.mailboxStatus
            const previousCode = previousCodeMap.get(row.id)
            const nextCode = nextStatus?.latestCode?.value ?? null
            return {
              ...row,
              mailboxStatus: nextStatus ?? null,
              mailboxCodeTone:
                nextCode && previousCode && nextCode !== previousCode ? 'idle' : row.mailboxCodeTone,
            }
          }),
        )
      } catch {
        // Ignore transient mailbox polling errors and keep batch OAuth usable.
      }
    }
    void poll()
    const timer = window.setInterval(() => {
      void poll()
    }, 5000)
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [batchRows, getOauthMailboxStatuses])

  useEffect(() => {
    setGroupDraftNotes((current) => {
      const nextEntries = Object.entries(current).filter(([groupName]) => !isExistingGroup(groups, groupName))
      if (nextEntries.length === Object.keys(current).length) {
        return current
      }
      return Object.fromEntries(nextEntries)
    })
  }, [groups])

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
    const previousDraftNote = resolvePendingGroupNoteForName(normalizedGroupName)
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
      if (previousDraftNote !== normalizedNote) {
        invalidatePendingOauthSessionsForDraftGroup(normalizedGroupName)
      }
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

  const appendBatchRow = () => {
    const nextId = `row-${batchRowIdRef.current++}`
    setBatchRows((current) => [...current, createBatchOauthRow(nextId, batchDefaultGroupName.trim())])
  }

  const scheduleSingleMailboxToneReset = useCallback(() => {
    if (oauthMailboxToneResetRef.current != null) {
      window.clearTimeout(oauthMailboxToneResetRef.current)
    }
    oauthMailboxToneResetRef.current = window.setTimeout(() => {
      setOauthMailboxTone('idle')
      oauthMailboxToneResetRef.current = null
    }, 1600)
  }, [])

  const updateBatchRow = (rowId: string, updater: (row: BatchOauthRow) => BatchOauthRow) => {
    setBatchRows((current) =>
      enforceBatchMotherDraftUniqueness(
        applyBatchMotherDraftRules(
          current.map((row) => (row.id === rowId ? updater(row) : row)),
          rowId,
        ),
      ),
    )
  }

  const scheduleBatchMailboxToneReset = useCallback((rowId: string) => {
    const currentTimer = batchMailboxToneResetRef.current[rowId]
    if (currentTimer != null) {
      window.clearTimeout(currentTimer)
    }
    batchMailboxToneResetRef.current[rowId] = window.setTimeout(() => {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxTone: 'idle',
      }))
      delete batchMailboxToneResetRef.current[rowId]
    }, 1600)
  }, [updateBatchRow])

  const invalidatePendingOauthSessionsForDraftGroup = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName)
      if (!normalizedGroupName) return

      if (session && session.status !== 'completed' && normalizeGroupName(oauthGroupName) === normalizedGroupName) {
        setSession(null)
        setSessionHint(t('accountPool.upstreamAccounts.oauth.regenerateRequired'))
        setOauthCallbackUrl('')
        setManualCopyOpen(false)
        setActionError(null)
      }

      const affectedRowIds = new Set(
        batchRows
          .filter(
            (row) =>
              row.session
              && row.session.status !== 'completed'
              && normalizeGroupName(row.groupName) === normalizedGroupName,
          )
          .map((row) => row.id),
      )
      if (affectedRowIds.size === 0) return

      setBatchRows((current) =>
        current.map((row) =>
          affectedRowIds.has(row.id)
            ? {
                ...row,
                callbackUrl: '',
                session: null,
                sessionHint: t('accountPool.upstreamAccounts.batchOauth.regenerateRequired'),
                actionError: null,
                busyAction: null,
              }
            : row,
        ),
      )
      setBatchManualCopyRowId((current) => (current && affectedRowIds.has(current) ? null : current))
    },
    [batchRows, oauthGroupName, session, t],
  )

  const removeBatchRow = (rowId: string) => {
    const mailboxSessionId = batchRows.find((row) => row.id === rowId)?.mailboxSession?.sessionId
    setBatchRows((current) => {
      const remaining = current.filter((row) => row.id !== rowId)
      return remaining.length > 0 ? remaining : [createBatchOauthRow(`row-${batchRowIdRef.current++}`, batchDefaultGroupName.trim())]
    })
    setBatchManualCopyRowId((current) => (current === rowId ? null : current))
    if (mailboxSessionId) {
      void removeOauthMailboxSession(mailboxSessionId).catch(() => undefined)
    }
  }

  const toggleBatchNoteExpanded = (rowId: string) => {
    updateBatchRow(rowId, (row) => ({
      ...row,
      noteExpanded: !row.noteExpanded,
    }))
  }

  const handleBatchMetadataChange = (
    rowId: string,
    field: 'displayName' | 'groupName' | 'note' | 'callbackUrl',
    value: string,
  ) => {
    updateBatchRow(rowId, (row) => {
      if (row.busyAction || row.session?.status === 'completed') {
        return row
      }
      const nextRow = {
        ...row,
        [field]: value,
      }
      if (field !== 'callbackUrl' && row.session && row.session.status !== 'completed') {
        return {
          ...nextRow,
          callbackUrl: '',
          session: null,
          sessionHint: t('accountPool.upstreamAccounts.batchOauth.regenerateRequired'),
          actionError: null,
          busyAction: null,
        }
      }
      return {
        ...nextRow,
        actionError: field === 'callbackUrl' ? null : row.actionError,
      }
    })
  }

  const handleBatchDefaultGroupChange = (value: string) => {
    setBatchDefaultGroupName((previousDefault) => {
      const previousTrimmed = previousDefault.trim()
      const nextTrimmed = value.trim()
      const affectedRowIds = new Set<string>()
      setBatchRows((current) =>
        enforceBatchMotherDraftUniqueness(
          current.map((row) => {
            if (row.busyAction || row.session?.status === 'completed') return row
            const inheritsDefault = !row.groupName.trim() || row.groupName === previousTrimmed
            if (!inheritsDefault) return row
            if (!row.session) {
              return { ...row, groupName: nextTrimmed }
            }
            affectedRowIds.add(row.id)
            return {
              ...row,
              groupName: nextTrimmed,
              callbackUrl: '',
              session: null,
              sessionHint: t('accountPool.upstreamAccounts.batchOauth.regenerateRequired'),
              actionError: null,
              busyAction: null,
            }
          }),
        ),
      )
      if (affectedRowIds.size > 0) {
        setBatchManualCopyRowId((current) => (current && affectedRowIds.has(current) ? null : current))
      }
      return value
    })
  }

  const handleTabChange = (tab: CreateTab) => {
    setActiveTab(tab)
    if (isRelinking) return
    const search = tab === 'oauth' ? '?mode=oauth' : `?mode=${tab}`
    navigate(`${location.pathname}${search}`, { replace: true })
  }

  const invalidateOauthSession = useCallback(() => {
    invalidatePendingSingleOauthSession(
      session,
      setSession,
      setSessionHint,
      setOauthCallbackUrl,
      setManualCopyOpen,
      setActionError,
      setOauthDuplicateWarning,
      t('accountPool.upstreamAccounts.oauth.regenerateRequired'),
    )
  }, [session, t])

  const notifyMotherChange = (updated: UpstreamAccountSummary) => {
    const nextItems = applyMotherUpdateToItems(items, updated)
    notifyMotherSwitches(items, nextItems)
  }

  const handleGenerateOauthMailbox = async () => {
    const previousSessionId = oauthMailboxSession?.sessionId
    setOauthMailboxBusy(true)
    setActionError(null)
    try {
      const response = await beginOauthMailboxSession()
      setOauthMailboxSession(response)
      setOauthMailboxInput(response.emailAddress)
      setOauthMailboxStatus(null)
      setOauthMailboxTone('idle')
      setOauthMailboxCodeTone('idle')
      invalidatePendingSingleOauthSession(
        session,
        setSession,
        setSessionHint,
        setOauthCallbackUrl,
        setManualCopyOpen,
        setActionError,
        setOauthDuplicateWarning,
        t('accountPool.upstreamAccounts.oauth.regenerateRequired'),
      )
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined)
      }
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setOauthMailboxBusy(false)
    }
  }

  const handleCopySingleMailbox = async () => {
    if (!oauthMailboxAddress) return
    const result = await copyText(oauthMailboxAddress, { preferExecCommand: true })
    if (!result.ok) return
    setOauthMailboxTone('copied')
    scheduleSingleMailboxToneReset()
  }

  const handleCopySingleMailboxCode = async () => {
    const value = oauthMailboxStatus?.latestCode?.value
    if (!value) return
    const result = await copyText(value, { preferExecCommand: true })
    if (result.ok) {
      setOauthMailboxCodeTone('copied')
    }
  }

  const handleCopySingleInvite = async () => {
    const value = oauthMailboxStatus?.invite?.copyValue
    if (!value) return
    await copyText(value, { preferExecCommand: true })
  }

  const handleGenerateOauthUrl = async () => {
    if (oauthDisplayNameConflict) {
      setActionError(null)
      return
    }
    setActionError(null)
    setSessionHint(null)
    setOauthDuplicateWarning(null)
    setBusyAction('oauth-generate')
    try {
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupName.trim() || undefined,
        note: oauthNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName) || undefined,
        accountId: relinkAccountId ?? undefined,
        tagIds: oauthTagIds,
        isMother: oauthIsMother,
        mailboxSessionId: oauthMailboxSession?.sessionId,
        generatedMailboxAddress: oauthMailboxSession?.emailAddress,
      })
      setSession(response)
      setManualCopyOpen(false)
      setOauthCallbackUrl('')
      setSessionHint(
        t('accountPool.upstreamAccounts.oauth.generated', {
          expiresAt: formatDateTime(response.expiresAt),
        }),
      )
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleCopyOauthUrl = async () => {
    if (!session?.authUrl) return
    setActionError(null)
    const result = await copyText(session.authUrl, {
      preferExecCommand: true,
    })
    if (result.ok) {
      setManualCopyOpen(false)
      setSessionHint(t('accountPool.upstreamAccounts.oauth.copied'))
      return
    }

    setManualCopyOpen(true)
    setSessionHint(t('accountPool.upstreamAccounts.oauth.copyFailed'))
  }

  const handleCompleteOauth = async () => {
    if (!session) return
    setActionError(null)
    setBusyAction('oauth-complete')
    try {
      const detail = await completeOauthLogin(session.loginId, {
        callbackUrl: oauthCallbackUrl.trim(),
        mailboxSessionId: oauthMailboxSession?.sessionId,
        generatedMailboxAddress: oauthMailboxSession?.emailAddress,
      })
      notifyMotherChange(detail)
      setSession({
        ...session,
        status: 'completed',
        accountId: detail.id,
        authUrl: null,
        redirectUri: null,
      })
      if (detail.duplicateInfo) {
        setOauthDuplicateWarning({
          accountId: detail.id,
          displayName: detail.displayName,
          peerAccountIds: detail.duplicateInfo.peerAccountIds,
          reasons: detail.duplicateInfo.reasons,
        })
      } else {
        navigate('/account-pool/upstream-accounts', {
          state: {
            selectedAccountId: detail.id,
            openDetail: true,
            duplicateWarning: null,
          },
        })
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      let latestSession: LoginSessionStatusResponse | null = null
      try {
        latestSession = await getLoginSession(session.loginId)
      } catch {
        latestSession = null
      }
      setSession((current) => latestSession ?? current)
      if (latestSession?.status === 'completed' && latestSession.accountId) {
        setActionError(null)
        emitUpstreamAccountsChanged()
        try {
          const detail = await fetchUpstreamAccountDetail(latestSession.accountId)
          notifyMotherChange(detail)
          if (detail.duplicateInfo) {
            setOauthDuplicateWarning({
              accountId: detail.id,
              displayName: detail.displayName,
              peerAccountIds: detail.duplicateInfo.peerAccountIds,
              reasons: detail.duplicateInfo.reasons,
            })
          } else {
            navigate('/account-pool/upstream-accounts', {
              state: {
                selectedAccountId: detail.id,
                openDetail: true,
                duplicateWarning: null,
              },
            })
          }
        } catch {
          navigate('/account-pool/upstream-accounts', {
            state: {
              selectedAccountId: latestSession.accountId,
              openDetail: true,
              duplicateWarning: null,
            },
          })
        }
        return
      }
      if (latestSession?.status === 'failed' || latestSession?.status === 'expired') {
        setOauthCallbackUrl('')
        setSessionHint(latestSession.error ?? message)
        setOauthDuplicateWarning(null)
      }
      setActionError(message)
    } finally {
      setBusyAction(null)
    }
  }

  const handleBatchGenerateMailbox = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    if (!row) return

    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxBusy: true,
      actionError: null,
    }))

    try {
      const response = await beginOauthMailboxSession()
      const previousSessionId = row.mailboxSession?.sessionId
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusy: false,
        mailboxSession: response,
        mailboxInput: response.emailAddress,
        mailboxStatus: null,
        mailboxTone: 'idle',
        mailboxCodeTone: 'idle',
        callbackUrl: '',
        session: null,
        sessionHint: null,
        actionError: null,
      }))
      if (previousSessionId && previousSessionId !== response.sessionId) {
        void removeOauthMailboxSession(previousSessionId).catch(() => undefined)
      }
    } catch (err) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        mailboxBusy: false,
        actionError: err instanceof Error ? err.message : String(err),
      }))
    }
  }

  const handleBatchCopyMailbox = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    const value = row?.mailboxSession?.emailAddress ?? row?.mailboxInput ?? ''
    if (!value) return
    const result = await copyText(value, { preferExecCommand: true })
    if (!result.ok) return
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxTone: 'copied',
    }))
    scheduleBatchMailboxToneReset(rowId)
  }

  const handleBatchCopyMailboxCode = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    const value = row?.mailboxStatus?.latestCode?.value
    if (!value) return
    const result = await copyText(value, { preferExecCommand: true })
    if (!result.ok) return
    updateBatchRow(rowId, (current) => ({
      ...current,
      mailboxCodeTone: 'copied',
    }))
  }

  const handleBatchGenerateOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    if (!row) return
    if (row.needsRefresh) return

    updateBatchRow(rowId, (current) => ({
      ...current,
      busyAction: 'generate',
      actionError: null,
    }))

    try {
      const response = await beginOauthLogin({
        displayName: row.displayName.trim() || undefined,
        groupName: row.groupName.trim() || undefined,
        note: row.note.trim() || undefined,
        tagIds: batchTagIds,
        groupNote: resolvePendingGroupNoteForName(row.groupName) || undefined,
        isMother: row.isMother,
        mailboxSessionId: row.mailboxSession?.sessionId,
        generatedMailboxAddress: row.mailboxSession?.emailAddress,
      })
      setBatchManualCopyRowId((current) => (current === rowId ? null : current))
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        callbackUrl: '',
        session: response,
        sessionHint: t('accountPool.upstreamAccounts.oauth.generated', {
          expiresAt: formatDateTime(response.expiresAt),
        }),
        needsRefresh: false,
        actionError: null,
      }))
    } catch (err) {
      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        actionError: err instanceof Error ? err.message : String(err),
      }))
    }
  }

  const handleBatchCopyOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    if (!row?.session?.authUrl) return

    updateBatchRow(rowId, (current) => ({
      ...current,
      actionError: null,
    }))

    const result = await copyText(row.session.authUrl, {
      preferExecCommand: true,
    })

    setBatchManualCopyRowId(result.ok ? null : rowId)

    updateBatchRow(rowId, (current) => ({
      ...current,
      sessionHint: result.ok
        ? t('accountPool.upstreamAccounts.oauth.copied')
        : t('accountPool.upstreamAccounts.batchOauth.copyInlineFallback'),
      actionError: result.ok ? null : t('accountPool.upstreamAccounts.batchOauth.copyInlineFallback'),
    }))
  }

  const handleBatchCompleteOauth = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    if (!row?.session) return

    updateBatchRow(rowId, (current) => ({
      ...current,
      busyAction: 'complete',
      actionError: null,
    }))

    try {
      const detail = await completeOauthLogin(row.session.loginId, {
        callbackUrl: row.callbackUrl.trim(),
        mailboxSessionId: row.mailboxSession?.sessionId,
        generatedMailboxAddress: row.mailboxSession?.emailAddress,
      })
      notifyMotherChange(detail)
      updateBatchRow(rowId, (current) => {
        const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse
        return {
          ...current,
          busyAction: null,
          session: {
            loginId: baseSession.loginId,
            status: 'completed',
            authUrl: null,
            redirectUri: null,
            expiresAt: baseSession.expiresAt,
            accountId: detail.id,
            error: baseSession.error ?? null,
          },
          sessionHint: t('accountPool.upstreamAccounts.batchOauth.completed', {
            name: detail.displayName || current.displayName || `#${detail.id}`,
          }),
          duplicateWarning: detail.duplicateInfo
            ? {
                accountId: detail.id,
                displayName: detail.displayName,
                peerAccountIds: detail.duplicateInfo.peerAccountIds,
                reasons: detail.duplicateInfo.reasons,
              }
            : null,
          needsRefresh: false,
          actionError: null,
          isMother: detail.isMother,
        }
      })
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      let latestSession: LoginSessionStatusResponse | null = null
      try {
        latestSession = await getLoginSession(row.session.loginId)
      } catch {
        latestSession = null
      }
      if (latestSession?.status === 'completed' && latestSession.accountId) {
        emitUpstreamAccountsChanged()
        try {
          const detail = await fetchUpstreamAccountDetail(latestSession.accountId)
          notifyMotherChange(detail)
          updateBatchRow(rowId, (current) => {
            const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse
            return {
              ...current,
              busyAction: null,
              session: {
                loginId: baseSession.loginId,
                status: 'completed',
                authUrl: null,
                redirectUri: null,
                expiresAt: baseSession.expiresAt,
                accountId: detail.id,
                error: null,
              },
              callbackUrl: '',
              sessionHint: t('accountPool.upstreamAccounts.batchOauth.completed', {
                name: detail.displayName || current.displayName || `#${detail.id}`,
              }),
              duplicateWarning: detail.duplicateInfo
                ? {
                    accountId: detail.id,
                    displayName: detail.displayName,
                    peerAccountIds: detail.duplicateInfo.peerAccountIds,
                    reasons: detail.duplicateInfo.reasons,
                  }
                : null,
              needsRefresh: false,
              actionError: null,
              isMother: detail.isMother,
            }
          })
        } catch {
          updateBatchRow(rowId, (current) => {
            const baseSession = (current.session ?? row.session) as LoginSessionStatusResponse
            return {
              ...current,
              busyAction: null,
              session: {
                loginId: baseSession.loginId,
                status: 'completed',
                authUrl: null,
                redirectUri: null,
                expiresAt: baseSession.expiresAt,
                accountId: latestSession.accountId,
                error: null,
              },
              callbackUrl: '',
              sessionHint: null,
              duplicateWarning: current.duplicateWarning,
              needsRefresh: true,
              actionError: t('accountPool.upstreamAccounts.batchOauth.completedNeedsRefresh'),
            }
          })
        }
        return
      }

      updateBatchRow(rowId, (current) => ({
        ...current,
        busyAction: null,
        session: latestSession ?? current.session,
        callbackUrl:
          latestSession?.status === 'failed' || latestSession?.status === 'expired' ? '' : current.callbackUrl,
        sessionHint:
          latestSession?.status === 'failed' || latestSession?.status === 'expired'
            ? latestSession.error ?? current.sessionHint
            : current.sessionHint,
        duplicateWarning:
          latestSession?.status === 'failed' || latestSession?.status === 'expired'
            ? null
            : current.duplicateWarning,
        needsRefresh: false,
        actionError: message,
      }))
    }
  }

  const handleCreateApiKey = async () => {
    if (apiKeyUpstreamBaseUrlError) return
    setActionError(null)
    setBusyAction('apiKey')
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        groupName: apiKeyGroupName.trim() || undefined,
        note: apiKeyNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(apiKeyGroupName) || undefined,
        apiKey: apiKeyValue.trim(),
        upstreamBaseUrl: apiKeyUpstreamBaseUrl.trim() || undefined,
        isMother: apiKeyIsMother,
        localPrimaryLimit: normalizeNumberInput(apiKeyPrimaryLimit),
        localSecondaryLimit: normalizeNumberInput(apiKeySecondaryLimit),
        localLimitUnit: apiKeyLimitUnit.trim() || 'requests',
        tagIds: apiKeyTagIds,
      })
      notifyMotherChange(response)
      navigate('/account-pool/upstream-accounts', {
        state: {
          selectedAccountId: response.id,
          openDetail: true,
        },
      })
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const oauthSessionActive = session?.status === 'pending'
  const batchCounts = batchRows.reduce(
    (accumulator, row) => {
      const status = batchRowStatus(row)
      accumulator.total += 1
      if (status === 'completed') accumulator.completed += 1
      else if (status === 'pending' || status === 'completedNeedsRefresh') accumulator.pending += 1
      else accumulator.draft += 1
      return accumulator
    },
    { total: 0, draft: 0, pending: 0, completed: 0 },
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

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <Button asChild variant="ghost" size="sm" className="mb-1 self-start px-0">
                <Link to="/account-pool/upstream-accounts">
                  <AppIcon name="arrow-left" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.upstreamAccounts.actions.backToList')}
                </Link>
              </Button>
              <h2 className="section-title">
                {isRelinking
                  ? t('accountPool.upstreamAccounts.createPage.relinkTitle')
                  : t('accountPool.upstreamAccounts.createPage.title')}
              </h2>
              <p className="section-description">
                {isRelinking
                  ? t('accountPool.upstreamAccounts.createPage.relinkDescription', {
                      name: relinkSummary?.displayName ?? t('accountPool.upstreamAccounts.unavailable'),
                    })
                  : t('accountPool.upstreamAccounts.createPage.description')}
              </p>
            </div>
            {isLoading ? <Spinner className="text-primary" /> : null}
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

          {error || actionError ? (
            <Alert variant="error">
              <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{actionError ?? error}</div>
            </Alert>
          ) : null}

          {session ? (
            <Alert variant={session.status === 'completed' ? 'success' : session.status === 'pending' ? 'info' : 'warning'}>
              <AppIcon
                name={session.status === 'completed' ? 'check-circle-outline' : 'link-variant-plus'}
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="space-y-1">
                <p className="font-medium">{t(`accountPool.upstreamAccounts.oauth.status.${session.status}`)}</p>
                <p className="text-sm opacity-90">{sessionHint ?? session.error ?? formatDateTime(session.expiresAt)}</p>
              </div>
            </Alert>
          ) : sessionHint ? (
            <Alert variant="warning">
              <AppIcon name="refresh-circle" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div className="text-sm">{sessionHint}</div>
            </Alert>
          ) : null}

          {!isRelinking ? (
            <div className="segment-group self-start" role="tablist" aria-label={t('accountPool.upstreamAccounts.createPage.tabsLabel')}>
              {(['oauth', 'batchOauth', 'apiKey'] as const).map((tab) => (
                <button
                  key={tab}
                  type="button"
                  role="tab"
                  aria-selected={activeTab === tab}
                  className="segment-button"
                  data-active={activeTab === tab}
                  onClick={() => handleTabChange(tab)}
                >
                  {tab === 'oauth'
                    ? t('accountPool.upstreamAccounts.createPage.tabs.oauth')
                    : tab === 'batchOauth'
                      ? t('accountPool.upstreamAccounts.createPage.tabs.batchOauth')
                      : t('accountPool.upstreamAccounts.createPage.tabs.apiKey')}
                </button>
              ))}
            </div>
          ) : null}

          <Card className="border-base-300/80 bg-base-100/72">
            <CardHeader className={cn(activeTab === 'batchOauth' && 'gap-3')}>
              {activeTab === 'batchOauth' ? (
                <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                  <div className="flex min-w-0 items-center gap-2">
                    <CardTitle className="shrink-0">
                      {t('accountPool.upstreamAccounts.batchOauth.createTitle')}
                    </CardTitle>
                    <Tooltip
                      content={buildActionTooltip(
                        t('accountPool.upstreamAccounts.batchOauth.createTitle'),
                        t('accountPool.upstreamAccounts.batchOauth.createDescription'),
                      )}
                    >
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-base-300/70 bg-base-100/72 text-base-content/55 transition hover:border-base-300 hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                        aria-label={t('accountPool.upstreamAccounts.batchOauth.createDescription')}
                      >
                        <AppIcon name="information-outline" className="h-4 w-4" aria-hidden />
                      </button>
                    </Tooltip>
                  </div>
                  <div className="flex w-full flex-wrap items-center justify-end gap-2 lg:w-auto lg:flex-nowrap lg:self-start">
                    <div className="flex min-w-0 items-center gap-2 sm:w-[24rem]">
                      <UpstreamAccountGroupCombobox
                        name="batchOauthDefaultGroupName"
                        value={batchDefaultGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t('accountPool.upstreamAccounts.batchOauth.defaultGroupPlaceholder')}
                        searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                        emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                        createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                        onValueChange={handleBatchDefaultGroupChange}
                        ariaLabel={t('accountPool.upstreamAccounts.batchOauth.defaultGroupLabel')}
                        className="min-w-0 flex-1"
                        triggerClassName="h-10 min-w-0 whitespace-nowrap rounded-lg"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={hasGroupNote(batchDefaultGroupName) ? 'secondary' : 'outline'}
                        className="h-10 w-10 shrink-0 rounded-full"
                        aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        onClick={() => openGroupNoteEditor(batchDefaultGroupName)}
                        disabled={!writesEnabled || !normalizeGroupName(batchDefaultGroupName)}
                      >
                        <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                    <div className="w-full lg:w-[24rem]">
                      <AccountTagField
                        tags={tagItems}
                        selectedTagIds={batchTagIds}
                        writesEnabled={writesEnabled}
                        pageCreatedTagIds={pageCreatedTagIds}
                        labels={tagFieldLabels}
                        onChange={setBatchTagIds}
                        onCreateTag={handleCreateTag}
                        onUpdateTag={updateTag}
                        onDeleteTag={handleDeleteTag}
                      />
                    </div>
                    <Button type="button" variant="secondary" onClick={appendBatchRow} disabled={!writesEnabled} className="h-10 shrink-0 rounded-lg">
                      <AppIcon name="playlist-plus" className="mr-2 h-4 w-4" aria-hidden />
                      {t('accountPool.upstreamAccounts.batchOauth.actions.addRow')}
                    </Button>
                  </div>
                </div>
              ) : (
                <>
                  <CardTitle>
                    {activeTab === 'oauth'
                      ? t('accountPool.upstreamAccounts.oauth.createTitle')
                      : t('accountPool.upstreamAccounts.apiKey.createTitle')}
                  </CardTitle>
                  <CardDescription>
                    {activeTab === 'oauth'
                      ? t('accountPool.upstreamAccounts.oauth.createDescription')
                      : t('accountPool.upstreamAccounts.apiKey.createDescription')}
                  </CardDescription>
                </>
              )}
            </CardHeader>
            <CardContent className={cn('grid gap-4', activeTab === 'apiKey' && 'md:grid-cols-2')}>
              {activeTab === 'oauth' ? (
                <>
                  <div className="field">
                    <div className="flex items-center gap-3">
                      <label htmlFor="oauth-display-name" className="field-label shrink-0">
                        {t('accountPool.upstreamAccounts.fields.displayName')}
                      </label>
                      <div className="flex min-w-0 flex-1 items-center justify-end gap-2">
                        <OauthMailboxChip
                          emailAddress={oauthMailboxAddress}
                          emptyLabel={t('accountPool.upstreamAccounts.oauth.mailboxEmpty')}
                          copyAriaLabel={t('accountPool.upstreamAccounts.actions.copyMailbox')}
                          copyHintLabel={t('accountPool.upstreamAccounts.actions.copyMailboxHint')}
                          copiedLabel={t('accountPool.upstreamAccounts.actions.copied')}
                          tone={oauthMailboxTone}
                          onCopy={() => void handleCopySingleMailbox()}
                        />
                        <Button
                          type="button"
                          size="sm"
                          variant="secondary"
                          className="h-7 shrink-0 rounded-full px-3"
                          onClick={() => void handleGenerateOauthMailbox()}
                          disabled={!writesEnabled || oauthMailboxBusy || session?.status === 'completed'}
                        >
                          {oauthMailboxBusy ? <Spinner size="sm" /> : t('accountPool.upstreamAccounts.actions.generateMailbox')}
                        </Button>
                      </div>
                    </div>
                    <div className="relative">
                      <Input
                        id="oauth-display-name"
                        name="oauthDisplayName"
                        value={oauthDisplayName}
                        aria-invalid={oauthDisplayNameConflict != null}
                        onChange={(event) => {
                          setOauthDisplayName(event.target.value)
                          invalidateOauthSession()
                        }}
                      />
                      {oauthDisplayNameConflict ? (
                        <FloatingFieldError
                          message={t('accountPool.upstreamAccounts.validation.displayNameDuplicate')}
                        />
                      ) : null}
                    </div>
                  </div>
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.groupName')}</span>
                    <div className="flex items-center gap-2">
                      <UpstreamAccountGroupCombobox
                        name="oauthGroupName"
                        value={oauthGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                        searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                        emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                        createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                        onValueChange={(value) => {
                          setOauthGroupName(value)
                          invalidateOauthSession()
                        }}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={hasGroupNote(oauthGroupName) ? 'secondary' : 'outline'}
                        className="shrink-0 rounded-full"
                        aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        onClick={() => openGroupNoteEditor(oauthGroupName)}
                        disabled={!writesEnabled || !normalizeGroupName(oauthGroupName) || oauthSessionActive}
                      >
                        <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                  </label>
                  <MotherAccountToggle
                    checked={oauthIsMother}
                    disabled={!writesEnabled}
                    label={t('accountPool.upstreamAccounts.mother.toggleLabel')}
                    description={t('accountPool.upstreamAccounts.mother.toggleDescription')}
                    onToggle={() => {
                      setOauthIsMother((current) => !current)
                      invalidateOauthSession()
                    }}
                  />
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                    <textarea
                      className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                      name="oauthNote"
                      value={oauthNote}
                      onChange={(event) => {
                        setOauthNote(event.target.value)
                        invalidateOauthSession()
                      }}
                    />
                  </label>
                  <AccountTagField
                    tags={tagItems}
                    selectedTagIds={oauthTagIds}
                    writesEnabled={writesEnabled}
                    pageCreatedTagIds={pageCreatedTagIds}
                    labels={tagFieldLabels}
                    onChange={setOauthTagIds}
                    onCreateTag={handleCreateTag}
                    onUpdateTag={updateTag}
                    onDeleteTag={handleDeleteTag}
                  />

                  <div className="grid gap-4 rounded-2xl border border-base-300/80 bg-base-100/72 p-4 sm:grid-cols-2">
                    <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="text-sm font-semibold text-base-content">
                            {t('accountPool.upstreamAccounts.oauth.codeCardTitle')}
                          </p>
                          <p className="mt-1 text-xs text-base-content/65">
                            {oauthMailboxStatus?.latestCode?.updatedAt
                              ? formatDateTime(oauthMailboxStatus.latestCode.updatedAt)
                              : t('accountPool.upstreamAccounts.oauth.codeCardEmpty')}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant={oauthMailboxCodeTone === 'copied' ? 'outline' : 'default'}
                          size="sm"
                          disabled={!oauthMailboxStatus?.latestCode?.value}
                          onClick={() => void handleCopySingleMailboxCode()}
                        >
                          <AppIcon name="content-copy" className="mr-1.5 h-4 w-4" aria-hidden />
                          {t('accountPool.upstreamAccounts.actions.copyCode')}
                        </Button>
                      </div>
                      <p className="mt-4 font-mono text-2xl font-semibold tracking-[0.24em] text-base-content">
                        {oauthMailboxStatus?.latestCode?.value ?? '—'}
                      </p>
                    </div>
                    <div className="rounded-2xl border border-base-300/70 bg-base-200/40 p-4">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <p className="text-sm font-semibold text-base-content">
                            {t('accountPool.upstreamAccounts.oauth.inviteCardTitle')}
                          </p>
                          <p className="mt-1 text-xs text-base-content/65">
                            {oauthMailboxStatus?.invite?.subject ?? t('accountPool.upstreamAccounts.oauth.inviteCardEmpty')}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant="secondary"
                          size="sm"
                          disabled={!oauthMailboxStatus?.invite?.copyValue}
                          onClick={() => void handleCopySingleInvite()}
                        >
                          <AppIcon name="content-copy" className="mr-1.5 h-4 w-4" aria-hidden />
                          {t('accountPool.upstreamAccounts.actions.copyInvite')}
                        </Button>
                      </div>
                      <div className="mt-4 flex items-center gap-3">
                        <Badge
                          variant={oauthMailboxStatus?.invited ? 'success' : 'secondary'}
                          className="rounded-full px-3 py-1 text-sm"
                        >
                          {oauthMailboxStatus?.invited
                            ? t('accountPool.upstreamAccounts.oauth.invitedState')
                            : t('accountPool.upstreamAccounts.oauth.notInvitedState')}
                        </Badge>
                        <span className="truncate text-sm text-base-content/70">
                          {oauthMailboxStatus?.invite?.copyValue ?? '—'}
                        </span>
                      </div>
                    </div>
                  </div>

                  <div className="rounded-2xl border border-base-300/80 bg-base-200/40 p-4 sm:p-5">
                    <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                      <div className="space-y-1">
                        <h3 className="text-sm font-semibold text-base-content">
                          {t('accountPool.upstreamAccounts.oauth.manualFlowTitle')}
                        </h3>
                        <p className="text-sm text-base-content/70">
                          {t('accountPool.upstreamAccounts.oauth.manualFlowDescription')}
                        </p>
                      </div>
                      <div className="flex shrink-0 flex-wrap gap-2">
                        <Button
                          type="button"
                          variant="secondary"
                          onClick={() => void handleGenerateOauthUrl()}
                          disabled={
                            busyAction === 'oauth-generate' ||
                            !writesEnabled ||
                            oauthDisplayNameConflict != null ||
                            session?.status === 'completed'
                          }
                        >
                          {busyAction === 'oauth-generate' ? (
                            <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                          ) : (
                            <AppIcon name="link-variant-plus" className="mr-2 h-4 w-4" aria-hidden />
                          )}
                          {session?.status === 'pending'
                            ? t('accountPool.upstreamAccounts.actions.regenerateOauthUrl')
                            : t('accountPool.upstreamAccounts.actions.generateOauthUrl')}
                        </Button>
                        <Popover open={manualCopyOpen} onOpenChange={setManualCopyOpen}>
                          <PopoverTrigger asChild>
                            <Button
                              type="button"
                              variant="secondary"
                              onClick={() => void handleCopyOauthUrl()}
                              disabled={!oauthSessionActive || !session?.authUrl}
                            >
                              <AppIcon name="content-copy" className="mr-2 h-4 w-4" aria-hidden />
                              {t('accountPool.upstreamAccounts.actions.copyOauthUrl')}
                            </Button>
                          </PopoverTrigger>
                          <PopoverContent align="end" sideOffset={10} className="w-[min(36rem,calc(100vw-2rem))] rounded-2xl border-base-300 bg-base-100 p-4 shadow-xl">
                            <div className="space-y-3">
                              <div className="space-y-1">
                                <p className="text-sm font-semibold text-base-content">
                                  {t('accountPool.upstreamAccounts.oauth.manualCopyTitle')}
                                </p>
                                <p className="text-sm text-base-content/65">
                                  {t('accountPool.upstreamAccounts.oauth.manualCopyDescription')}
                                </p>
                              </div>
                              <textarea
                                ref={manualCopyFieldRef}
                                readOnly
                                value={session?.authUrl ?? ''}
                                className="min-h-28 w-full rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm focus-visible:outline-none"
                                onClick={(event) => selectAllReadonlyText(event.currentTarget)}
                                onFocus={(event) => selectAllReadonlyText(event.currentTarget)}
                              />
                            </div>
                          </PopoverContent>
                        </Popover>
                      </div>
                    </div>

                    <div className="mt-4 grid gap-4">
                      <div className="grid gap-4">
                        <label className="field">
                          <span className="field-label">{t('accountPool.upstreamAccounts.oauth.callbackUrlLabel')}</span>
                          <textarea
                            name="oauthCallbackUrl"
                            value={oauthCallbackUrl}
                            onChange={(event) => setOauthCallbackUrl(event.target.value)}
                            placeholder={t('accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder')}
                            className="min-h-24 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                          />
                          <span className="text-xs text-base-content/60">
                            {t('accountPool.upstreamAccounts.oauth.callbackUrlDescription')}
                          </span>
                        </label>
                      </div>
                    </div>
                  </div>

                  <div className="flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">{t('accountPool.upstreamAccounts.actions.cancel')}</Link>
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleCompleteOauth()}
                      disabled={
                        !oauthSessionActive ||
                        !oauthCallbackUrl.trim() ||
                        busyAction === 'oauth-complete' ||
                        !writesEnabled ||
                        oauthDisplayNameConflict != null
                      }
                    >
                      {busyAction === 'oauth-complete' ? (
                        <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                      ) : (
                        <AppIcon name="check-decagram-outline" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.completeOauth')}
                    </Button>
                    {oauthDuplicateWarning ? (
                      <DuplicateWarningPopover
                        duplicateWarning={oauthDuplicateWarning}
                        summaryTitle={t('accountPool.upstreamAccounts.duplicate.compactTitle')}
                        summaryBody={t('accountPool.upstreamAccounts.duplicate.compactBody', {
                          reasons: formatDuplicateReasons(oauthDuplicateWarning),
                          peers: oauthDuplicateWarning.peerAccountIds.join(', '),
                        })}
                        openDetailsLabel={t('accountPool.upstreamAccounts.actions.openDetails')}
                        onOpenDetails={openDuplicateDetailDialog}
                      />
                    ) : null}
                  </div>
                </>
              ) : activeTab === 'batchOauth' ? (
                <>
                  <div className="space-y-3">
                    <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t('accountPool.upstreamAccounts.batchOauth.summary.total')}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">{batchCounts.total}</p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t('accountPool.upstreamAccounts.batchOauth.summary.draft')}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">{batchCounts.draft}</p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t('accountPool.upstreamAccounts.batchOauth.summary.pending')}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">{batchCounts.pending}</p>
                      </div>
                      <div className="rounded-2xl border border-base-300/80 bg-base-100/78 px-4 py-3">
                        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/50">
                          {t('accountPool.upstreamAccounts.batchOauth.summary.completed')}
                        </p>
                        <p className="mt-1 text-xl font-semibold text-base-content">{batchCounts.completed}</p>
                      </div>
                    </div>

                    <div className="overflow-hidden rounded-[1.35rem] border border-base-300/80 bg-base-100/92 shadow-sm shadow-base-300/20">
                      <table className="w-full table-fixed text-sm">
                        <colgroup>
                          <col className="w-14" />
                          <col className="w-[44%]" />
                          <col className="w-[56%]" />
                        </colgroup>
                        <thead className="bg-base-100/86">
                          <tr className="border-b border-base-300/80">
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              #
                            </th>
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              {t('accountPool.upstreamAccounts.batchOauth.tableAccountColumn')}
                            </th>
                            <th className="px-3 py-3 text-left text-[11px] font-semibold uppercase tracking-[0.16em] text-base-content/55">
                              {t('accountPool.upstreamAccounts.batchOauth.tableFlowColumn')}
                            </th>
                          </tr>
                        </thead>
                        <tbody>
                          {batchRows.map((row, index) => {
                            const status = batchRowStatus(row)
                            const statusDetail = batchRowStatusDetail(row)
                            const duplicateNameError = batchDisplayNameError(row)
                            const isCompleted = status === 'completed'
                            const isRecoveredNeedsRefresh = status === 'completedNeedsRefresh'
                            const isPending = status === 'pending'
                            const isBusy = row.busyAction != null
                            const rowLocked = isBusy || isCompleted || isRecoveredNeedsRefresh
                            const authUrl = row.session?.authUrl ?? ''
                            const rowMailboxAddress = row.mailboxSession?.emailAddress ?? row.mailboxInput
                            const rowInvited = row.mailboxStatus?.invited
                            return (
                              <tr
                                key={row.id}
                                data-testid={`batch-oauth-row-${row.id}`}
                                className="align-top border-b border-base-300/70 last:border-b-0"
                              >
                                <td className="px-3 py-4">
                                  <span
                                    className={cn(
                                      'inline-flex h-8 min-w-8 items-center justify-center rounded-full border px-2 text-sm font-semibold',
                                      rowInvited
                                        ? 'border-success/40 bg-success/12 text-success'
                                        : 'border-base-300/80 text-base-content/72',
                                    )}
                                  >
                                    {index + 1}
                                  </span>
                                </td>
                                <td className="px-3 py-4">
                                  <div className="grid gap-3">
                                    <div className="field min-w-0 gap-2 whitespace-nowrap">
                                      <div className="flex items-center gap-3">
                                        <label
                                          htmlFor={`batch-oauth-display-name-${row.id}`}
                                          className="field-label shrink-0"
                                        >
                                          {t('accountPool.upstreamAccounts.fields.displayName')}
                                        </label>
                                        <div className="flex min-w-0 flex-1 items-center justify-end gap-2">
                                          <OauthMailboxChip
                                            emailAddress={rowMailboxAddress}
                                            emptyLabel={t('accountPool.upstreamAccounts.oauth.mailboxEmpty')}
                                            copyAriaLabel={t('accountPool.upstreamAccounts.actions.copyMailbox')}
                                            copyHintLabel={t('accountPool.upstreamAccounts.actions.copyMailboxHint')}
                                            copiedLabel={t('accountPool.upstreamAccounts.actions.copied')}
                                            tone={row.mailboxTone}
                                            onCopy={() => void handleBatchCopyMailbox(row.id)}
                                          />
                                          <Button
                                            type="button"
                                            size="sm"
                                            variant="secondary"
                                            className="h-7 shrink-0 rounded-full px-3"
                                            onClick={() => void handleBatchGenerateMailbox(row.id)}
                                            disabled={!writesEnabled || rowLocked || row.mailboxBusy}
                                          >
                                            {row.mailboxBusy ? <Spinner size="sm" /> : t('accountPool.upstreamAccounts.actions.generateMailbox')}
                                          </Button>
                                        </div>
                                      </div>
                                      <div className="relative">
                                        <Input
                                          id={`batch-oauth-display-name-${row.id}`}
                                          name={`batchOauthDisplayName-${row.id}`}
                                          value={row.displayName}
                                          disabled={rowLocked}
                                          aria-invalid={duplicateNameError != null}
                                          className="min-w-0"
                                          onChange={(event) => handleBatchMetadataChange(row.id, 'displayName', event.target.value)}
                                        />
                                        {duplicateNameError ? (
                                          <FloatingFieldError message={duplicateNameError} />
                                        ) : null}
                                      </div>
                                    </div>
                                    <label className="field min-w-0 gap-2 whitespace-nowrap">
                                      <span className="field-label">{t('accountPool.upstreamAccounts.fields.groupName')}</span>
                                      <div className="flex min-w-0 items-center gap-2">
                                        <UpstreamAccountGroupCombobox
                                          name={`batchOauthGroupName-${row.id}`}
                                          value={row.groupName}
                                          suggestions={groupSuggestions}
                                          placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                                          searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                                          emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                                          createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                                          onValueChange={(value) => handleBatchMetadataChange(row.id, 'groupName', value)}
                                          disabled={rowLocked}
                                          className="min-w-0 flex-1"
                                          triggerClassName="min-w-0 whitespace-nowrap"
                                        />
                                        <Button
                                          type="button"
                                          size="icon"
                                          variant={hasGroupNote(row.groupName) ? 'secondary' : 'outline'}
                                          className="h-10 w-10 shrink-0 rounded-full"
                                          aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                          title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                          onClick={() => openGroupNoteEditor(row.groupName)}
                                          disabled={!writesEnabled || !normalizeGroupName(row.groupName)}
                                        >
                                          <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
                                        </Button>
                                      </div>
                                    </label>
                                    {row.noteExpanded ? (
                                      <label className="field min-w-0 gap-2 whitespace-nowrap">
                                        <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                                        <Input
                                          name={`batchOauthNote-${row.id}`}
                                          value={row.note}
                                          disabled={rowLocked}
                                        className="min-w-0"
                                          onChange={(event) => handleBatchMetadataChange(row.id, 'note', event.target.value)}
                                        />
                                      </label>
                                    ) : null}
                                  </div>
                                </td>
                                <td className="px-3 py-4">
                                  <div className="grid gap-3">
                                    <label className="field min-w-0 gap-2 whitespace-nowrap">
                                      <span className="field-label">{t('accountPool.upstreamAccounts.oauth.callbackUrlLabel')}</span>
                                      <Input
                                        name={`batchOauthCallbackUrl-${row.id}`}
                                        value={row.callbackUrl}
                                        disabled={rowLocked}
                                        placeholder={t('accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder')}
                                        className="min-w-0"
                                        onChange={(event) => handleBatchMetadataChange(row.id, 'callbackUrl', event.target.value)}
                                      />
                                    </label>
                                    <div className="flex items-center gap-3">
                                      <div className="flex flex-wrap items-center gap-2">
                                        <Tooltip
                                          content={buildActionTooltip(
                                            isPending
                                              ? t('accountPool.upstreamAccounts.batchOauth.tooltip.regenerateTitle')
                                              : t('accountPool.upstreamAccounts.batchOauth.tooltip.generateTitle'),
                                            isPending
                                              ? t('accountPool.upstreamAccounts.batchOauth.tooltip.regenerateBody')
                                              : t('accountPool.upstreamAccounts.batchOauth.tooltip.generateBody'),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant={isPending ? 'destructive' : 'default'}
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={isPending
                                              ? t('accountPool.upstreamAccounts.actions.regenerateOauthUrl')
                                              : t('accountPool.upstreamAccounts.actions.generateOauthUrl')}
                                            onClick={() => void handleBatchGenerateOauthUrl(row.id)}
                                            disabled={isBusy || isCompleted || isRecoveredNeedsRefresh || !writesEnabled}
                                          >
                                            {row.busyAction === 'generate' ? (
                                              <Spinner size="sm" />
                                            ) : (
                                              <AppIcon name={isPending ? 'refresh' : 'link-variant-plus'} className="h-4 w-4" aria-hidden />
                                            )}
                                          </Button>
                                        </Tooltip>
                                        {row.mailboxSession ? (
                                          <Tooltip
                                            content={buildActionTooltip(
                                              t('accountPool.upstreamAccounts.batchOauth.tooltip.copyCodeTitle'),
                                              row.mailboxStatus?.latestCode?.value ?? t('accountPool.upstreamAccounts.batchOauth.codeMissing'),
                                            )}
                                          >
                                            <Button
                                              type="button"
                                              size="icon"
                                              variant={batchMailboxCodeVariant(row)}
                                              className="h-9 w-9 shrink-0 rounded-full"
                                              aria-label={t('accountPool.upstreamAccounts.actions.copyCode')}
                                              onClick={() => void handleBatchCopyMailboxCode(row.id)}
                                              disabled={!row.mailboxStatus?.latestCode?.value}
                                            >
                                              <AppIcon name="content-copy" className="h-4 w-4" aria-hidden />
                                            </Button>
                                          </Tooltip>
                                        ) : null}
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.copyTitle'),
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.copyBody'),
                                          )}
                                        >
                                          <Popover
                                            open={batchManualCopyRowId === row.id}
                                            onOpenChange={(nextOpen) => {
                                              setBatchManualCopyRowId(nextOpen ? row.id : null)
                                            }}
                                          >
                                            <PopoverAnchor asChild>
                                              <Button
                                                type="button"
                                                size="icon"
                                                variant={authUrl ? 'default' : 'secondary'}
                                                className="h-9 w-9 shrink-0 rounded-full"
                                                aria-label={t('accountPool.upstreamAccounts.actions.copyOauthUrl')}
                                                onClick={() => void handleBatchCopyOauthUrl(row.id)}
                                                disabled={!authUrl || isBusy}
                                              >
                                                <AppIcon name="content-copy" className="h-4 w-4" aria-hidden />
                                              </Button>
                                            </PopoverAnchor>
                                            <PopoverContent
                                              align="start"
                                              sideOffset={10}
                                              className="w-[min(32rem,calc(100vw-2rem))] rounded-2xl border-base-300 bg-base-100 p-4 shadow-xl"
                                            >
                                              <div className="space-y-3">
                                                <div className="space-y-1">
                                                  <p className="text-sm font-semibold text-base-content">
                                                    {t('accountPool.upstreamAccounts.oauth.manualCopyTitle')}
                                                  </p>
                                                  <p className="text-sm text-base-content/65">
                                                    {t('accountPool.upstreamAccounts.oauth.manualCopyDescription')}
                                                  </p>
                                                </div>
                                                <textarea
                                                  ref={batchManualCopyRowId === row.id ? batchManualCopyFieldRef : undefined}
                                                  readOnly
                                                  value={authUrl}
                                                  className="min-h-28 w-full rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content shadow-sm focus-visible:outline-none"
                                                  onClick={(event) => selectAllReadonlyText(event.currentTarget)}
                                                  onFocus={(event) => selectAllReadonlyText(event.currentTarget)}
                                                />
                                              </div>
                                            </PopoverContent>
                                          </Popover>
                                        </Tooltip>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.noteTitle'),
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.noteBody'),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant={row.noteExpanded || row.note.trim() ? 'secondary' : 'ghost'}
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={row.noteExpanded
                                              ? t('accountPool.upstreamAccounts.batchOauth.actions.collapseNote')
                                              : t('accountPool.upstreamAccounts.batchOauth.actions.expandNote')}
                                            onClick={() => toggleBatchNoteExpanded(row.id)}
                                          >
                                            <AppIcon
                                              name={row.noteExpanded ? 'chevron-up' : 'note-text-outline'}
                                              className="h-4 w-4"
                                              aria-hidden
                                            />
                                          </Button>
                                        </Tooltip>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.completeTitle'),
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.completeBody'),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={t('accountPool.upstreamAccounts.actions.completeOauth')}
                                            onClick={() => void handleBatchCompleteOauth(row.id)}
                                            disabled={
                                              !writesEnabled ||
                                              isBusy ||
                                              isCompleted ||
                                              !isPending ||
                                              !row.callbackUrl.trim() ||
                                              duplicateNameError != null
                                            }
                                          >
                                            {row.busyAction === 'complete' ? (
                                              <Spinner size="sm" />
                                            ) : (
                                              <AppIcon name="check-bold" className="h-4 w-4" aria-hidden />
                                            )}
                                          </Button>
                                        </Tooltip>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.motherTitle'),
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.motherBody'),
                                          )}
                                        >
                                          <MotherAccountToggle
                                            checked={row.isMother}
                                            disabled={rowLocked || !writesEnabled}
                                            iconOnly
                                            label={t('accountPool.upstreamAccounts.mother.badge')}
                                            ariaLabel={t('accountPool.upstreamAccounts.batchOauth.actions.toggleMother')}
                                            onToggle={() =>
                                              updateBatchRow(row.id, (current) => ({
                                                ...current,
                                                isMother: !current.isMother,
                                              }))
                                            }
                                          />
                                        </Tooltip>
                                        {row.duplicateWarning ? (
                                          <DuplicateWarningPopover
                                            duplicateWarning={row.duplicateWarning}
                                            summaryTitle={t('accountPool.upstreamAccounts.duplicate.compactTitle')}
                                            summaryBody={t('accountPool.upstreamAccounts.duplicate.compactBody', {
                                              reasons: formatDuplicateReasons(row.duplicateWarning),
                                              peers: row.duplicateWarning.peerAccountIds.join(', '),
                                            })}
                                            openDetailsLabel={t('accountPool.upstreamAccounts.actions.openDetails')}
                                            onOpenDetails={openDuplicateDetailDialog}
                                          />
                                        ) : null}
                                      </div>
                                      <div className="ml-auto flex shrink-0 items-center gap-2">
                                        <Badge variant={batchStatusVariant(status)}>
                                          {t(`accountPool.upstreamAccounts.batchOauth.status.${status}`)}
                                        </Badge>
                                        <Tooltip
                                          content={buildActionTooltip(
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.removeTitle'),
                                            t('accountPool.upstreamAccounts.batchOauth.tooltip.removeBody'),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant="destructive"
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={t('accountPool.upstreamAccounts.batchOauth.actions.removeRow')}
                                            onClick={() => removeBatchRow(row.id)}
                                            disabled={isBusy || isCompleted}
                                          >
                                            <AppIcon name="delete-outline" className="h-4 w-4" aria-hidden />
                                          </Button>
                                        </Tooltip>
                                      </div>
                                    </div>
                                    <p className="text-xs leading-5 text-base-content/65">
                                      {statusDetail ?? t('accountPool.upstreamAccounts.batchOauth.statusDetail.draft')}
                                    </p>
                                  </div>
                                </td>
                              </tr>
                            )
                          })}
                        </tbody>
                      </table>
                    </div>
                  </div>

                  <p className="text-sm text-base-content/65">{t('accountPool.upstreamAccounts.batchOauth.footerHint')}</p>

                </>
              ) : (
                <>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.displayName')}</span>
                    <div className="relative">
                      <Input
                        name="apiKeyDisplayName"
                        value={apiKeyDisplayName}
                        aria-invalid={apiKeyDisplayNameConflict != null}
                        onChange={(event) => setApiKeyDisplayName(event.target.value)}
                      />
                      {apiKeyDisplayNameConflict ? (
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
                        name="apiKeyGroupName"
                        value={apiKeyGroupName}
                        suggestions={groupSuggestions}
                        placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                        searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                        emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                        createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                        onValueChange={setApiKeyGroupName}
                        className="min-w-0 flex-1"
                      />
                      <Button
                        type="button"
                        size="icon"
                        variant={hasGroupNote(apiKeyGroupName) ? 'secondary' : 'outline'}
                        className="shrink-0 rounded-full"
                        aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                        onClick={() => openGroupNoteEditor(apiKeyGroupName)}
                        disabled={!writesEnabled || !normalizeGroupName(apiKeyGroupName)}
                      >
                        <AppIcon name="file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                  </label>
                  <div className="md:col-span-2">
                    <MotherAccountToggle
                      checked={apiKeyIsMother}
                      disabled={!writesEnabled}
                      label={t('accountPool.upstreamAccounts.mother.toggleLabel')}
                      description={t('accountPool.upstreamAccounts.mother.toggleDescription')}
                      onToggle={() => setApiKeyIsMother((current) => !current)}
                    />
                  </div>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.apiKey')}</span>
                    <Input
                      name="apiKeyValue"
                      value={apiKeyValue}
                      onChange={(event) => setApiKeyValue(event.target.value)}
                    />
                  </label>
                  <label className="field md:col-span-2">
                    <FormFieldFeedback
                      label={t('accountPool.upstreamAccounts.fields.upstreamBaseUrl')}
                      message={apiKeyUpstreamBaseUrlError}
                      messageClassName="md:max-w-[min(30rem,calc(100%-9rem))]"
                    />
                    <div className="relative">
                      <Input
                        name="apiKeyUpstreamBaseUrl"
                        value={apiKeyUpstreamBaseUrl}
                        onChange={(event) => setApiKeyUpstreamBaseUrl(event.target.value)}
                        placeholder={t('accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder')}
                        autoCapitalize="none"
                        spellCheck={false}
                        aria-invalid={apiKeyUpstreamBaseUrlError ? 'true' : 'false'}
                        className={cn(apiKeyUpstreamBaseUrlError ? 'border-error/70 focus-visible:ring-error' : '')}
                      />
                    </div>
                  </label>
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.primaryLimit')}</span>
                    <Input
                      name="apiKeyPrimaryLimit"
                      value={apiKeyPrimaryLimit}
                      onChange={(event) => setApiKeyPrimaryLimit(event.target.value)}
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.secondaryLimit')}</span>
                    <Input
                      name="apiKeySecondaryLimit"
                      value={apiKeySecondaryLimit}
                      onChange={(event) => setApiKeySecondaryLimit(event.target.value)}
                    />
                  </label>
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.limitUnit')}</span>
                    <Input
                      name="apiKeyLimitUnit"
                      value={apiKeyLimitUnit}
                      onChange={(event) => setApiKeyLimitUnit(event.target.value)}
                    />
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                    <textarea
                      className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                      name="apiKeyNote"
                      value={apiKeyNote}
                      onChange={(event) => setApiKeyNote(event.target.value)}
                    />
                  </label>
                  <div className="md:col-span-2">
                    <AccountTagField
                      tags={tagItems}
                      selectedTagIds={apiKeyTagIds}
                      writesEnabled={writesEnabled}
                      pageCreatedTagIds={pageCreatedTagIds}
                      labels={tagFieldLabels}
                      onChange={setApiKeyTagIds}
                      onCreateTag={handleCreateTag}
                      onUpdateTag={updateTag}
                      onDeleteTag={handleDeleteTag}
                    />
                  </div>
                  <div className="md:col-span-2 flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">{t('accountPool.upstreamAccounts.actions.cancel')}</Link>
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleCreateApiKey()}
                      disabled={
                        busyAction === 'apiKey' ||
                        !writesEnabled ||
                        apiKeyDisplayNameConflict != null ||
                        Boolean(apiKeyUpstreamBaseUrlError)
                      }
                    >
                      {busyAction === 'apiKey' ? (
                        <AppIcon name="loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                      ) : (
                        <AppIcon name="content-save-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.createApiKey')}
                    </Button>
                  </div>
                </>
              )}
            </CardContent>
          </Card>
        </div>
      </section>
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
      <DuplicateAccountDetailDialog
        open={duplicateDetailOpen}
        detail={duplicateDetail}
        isLoading={duplicateDetailLoading}
        onClose={() => {
          setDuplicateDetailOpen(false)
          setDuplicateDetail(null)
        }}
        title={t('accountPool.upstreamAccounts.detailTitle')}
        description={t('accountPool.upstreamAccounts.detailEmptyDescription')}
        duplicateLabel={t('accountPool.upstreamAccounts.duplicate.badge')}
        closeLabel={t('accountPool.upstreamAccounts.actions.closeDetails')}
        formatDuplicateReasons={formatDuplicateReasons}
        statusLabel={accountStatusLabel}
        kindLabel={accountKindLabel}
        fieldLabels={{
          groupName: t('accountPool.upstreamAccounts.fields.groupName'),
          email: t('accountPool.upstreamAccounts.fields.email'),
          accountId: t('accountPool.upstreamAccounts.fields.accountId'),
          userId: t('accountPool.upstreamAccounts.fields.userId'),
          lastSuccessSync: t('accountPool.upstreamAccounts.fields.lastSuccessSync'),
        }}
      />
    </div>
  )
}
