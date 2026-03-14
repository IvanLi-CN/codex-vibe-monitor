import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { Alert } from '../../components/ui/alert'
import { Badge } from '../../components/ui/badge'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { Input } from '../../components/ui/input'
import { Popover, PopoverAnchor, PopoverContent, PopoverTrigger } from '../../components/ui/popover'
import { Spinner } from '../../components/ui/spinner'
import { Tooltip } from '../../components/ui/tooltip'
import { UpstreamAccountGroupCombobox } from '../../components/UpstreamAccountGroupCombobox'
import { UpstreamAccountGroupNoteDialog } from '../../components/UpstreamAccountGroupNoteDialog'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import type { LoginSessionStatusResponse } from '../../lib/api'
import { copyText, selectAllReadonlyText } from '../../lib/clipboard'
import { isExistingGroup, normalizeGroupName, resolveGroupNote } from '../../lib/upstreamAccountGroups'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

type CreateTab = 'oauth' | 'batchOauth' | 'apiKey'
type BatchOauthBusyAction = 'generate' | 'complete' | null
type GroupNoteEditorState = {
  open: boolean
  groupName: string
  note: string
  existing: boolean
}

type BatchOauthRow = {
  id: string
  displayName: string
  groupName: string
  note: string
  noteExpanded: boolean
  callbackUrl: string
  session: LoginSessionStatusResponse | null
  sessionHint: string | null
  actionError: string | null
  busyAction: BatchOauthBusyAction
}

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
    note: '',
    noteExpanded: false,
    callbackUrl: '',
    session: null,
    sessionHint: null,
    actionError: null,
    busyAction: null,
  }
}

function batchStatusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'completed') return 'success'
  if (status === 'pending') return 'warning'
  if (status === 'failed' || status === 'expired') return 'error'
  return 'secondary'
}

function batchRowStatus(row: BatchOauthRow) {
  return row.session?.status ?? 'draft'
}

function batchRowStatusDetail(row: BatchOauthRow) {
  if (row.actionError) return row.actionError
  if (row.sessionHint) return row.sessionHint
  if (row.session?.error) return row.session.error
  if (row.session?.expiresAt) return formatDateTime(row.session.expiresAt)
  return null
}

function buildActionTooltip(title: string, description: string) {
  return (
    <div className="space-y-1">
      <p className="font-semibold text-base-content">{title}</p>
      <p className="leading-5 text-base-content/70">{description}</p>
    </div>
  )
}

export default function UpstreamAccountCreatePage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const location = useLocation()
  const {
    items,
    groups,
    writesEnabled,
    isLoading,
    error,
    beginOauthLogin,
    getLoginSession,
    completeOauthLogin,
    createApiKeyAccount,
    saveGroupNote,
  } = useUpstreamAccounts()

  const relinkAccountId = useMemo(() => parseAccountId(location.search), [location.search])
  const relinkSummary = useMemo(
    () => (relinkAccountId == null ? null : items.find((item) => item.id === relinkAccountId) ?? null),
    [items, relinkAccountId],
  )
  const isRelinking = relinkAccountId != null

  const [activeTab, setActiveTab] = useState<CreateTab>(() => (isRelinking ? 'oauth' : parseCreateMode(location.search)))
  const [oauthDisplayName, setOauthDisplayName] = useState('')
  const [oauthGroupName, setOauthGroupName] = useState('')
  const [oauthNote, setOauthNote] = useState('')
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState('')
  const [apiKeyDisplayName, setApiKeyDisplayName] = useState('')
  const [apiKeyGroupName, setApiKeyGroupName] = useState('')
  const [apiKeyNote, setApiKeyNote] = useState('')
  const [apiKeyValue, setApiKeyValue] = useState('')
  const [apiKeyPrimaryLimit, setApiKeyPrimaryLimit] = useState('')
  const [apiKeySecondaryLimit, setApiKeySecondaryLimit] = useState('')
  const [apiKeyLimitUnit, setApiKeyLimitUnit] = useState('requests')
  const [session, setSession] = useState<LoginSessionStatusResponse | null>(null)
  const [sessionHint, setSessionHint] = useState<string | null>(null)
  const [actionError, setActionError] = useState<string | null>(null)
  const [busyAction, setBusyAction] = useState<string | null>(null)
  const [manualCopyOpen, setManualCopyOpen] = useState(false)
  const [batchManualCopyRowId, setBatchManualCopyRowId] = useState<string | null>(null)
  const [batchDefaultGroupName, setBatchDefaultGroupName] = useState('')
  const [batchRows, setBatchRows] = useState<BatchOauthRow[]>(() =>
    Array.from({ length: 5 }, (_, index) => createBatchOauthRow(`row-${index + 1}`)),
  )
  const [groupDraftNotes, setGroupDraftNotes] = useState<Record<string, string>>({})
  const [groupNoteEditor, setGroupNoteEditor] = useState<GroupNoteEditorState>({
    open: false,
    groupName: '',
    note: '',
    existing: false,
  })
  const [groupNoteBusy, setGroupNoteBusy] = useState(false)
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null)
  const batchRowIdRef = useRef(6)
  const manualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null)
  const batchManualCopyFieldRef = useRef<HTMLTextAreaElement | null>(null)

  const groupSuggestions = Array.from(
    new Set(
      [...items.map((item) => item.groupName?.trim()), ...groups.map((group) => group.groupName.trim())]
        .filter((value): value is string => Boolean(value)),
    ),
  ).sort((left, right) => left.localeCompare(right))

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

  const appendBatchRow = () => {
    const nextId = `row-${batchRowIdRef.current++}`
    setBatchRows((current) => [...current, createBatchOauthRow(nextId, batchDefaultGroupName.trim())])
  }

  const updateBatchRow = (rowId: string, updater: (row: BatchOauthRow) => BatchOauthRow) => {
    setBatchRows((current) => current.map((row) => (row.id === rowId ? updater(row) : row)))
  }

  const removeBatchRow = (rowId: string) => {
    setBatchRows((current) => {
      const remaining = current.filter((row) => row.id !== rowId)
      return remaining.length > 0 ? remaining : [createBatchOauthRow(`row-${batchRowIdRef.current++}`, batchDefaultGroupName.trim())]
    })
    setBatchManualCopyRowId((current) => (current === rowId ? null : current))
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
      setBatchRows((current) =>
        current.map((row) => {
          if (row.busyAction || row.session?.status === 'completed') return row
          const inheritsDefault = !row.groupName.trim() || row.groupName === previousTrimmed
          return inheritsDefault ? { ...row, groupName: nextTrimmed } : row
        }),
      )
      return value
    })
  }

  const handleTabChange = (tab: CreateTab) => {
    setActiveTab(tab)
    if (isRelinking) return
    const search = tab === 'oauth' ? '?mode=oauth' : `?mode=${tab}`
    navigate(`${location.pathname}${search}`, { replace: true })
  }

  const handleGenerateOauthUrl = async () => {
    setActionError(null)
    setSessionHint(null)
    setBusyAction('oauth-generate')
    try {
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupName.trim() || undefined,
        note: oauthNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(oauthGroupName) || undefined,
        accountId: relinkAccountId ?? undefined,
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
      })
      setSession({
        ...session,
        status: 'completed',
        accountId: detail.id,
        authUrl: null,
        redirectUri: null,
      })
      navigate('/account-pool/upstream-accounts', {
        state: {
          selectedAccountId: detail.id,
          openDetail: true,
        },
      })
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleBatchGenerateOauthUrl = async (rowId: string) => {
    const row = batchRows.find((item) => item.id === rowId)
    if (!row) return

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
        groupNote: resolvePendingGroupNoteForName(row.groupName) || undefined,
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
      })
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
          actionError: null,
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
        actionError: message,
      }))
    }
  }

  const handleCreateApiKey = async () => {
    setActionError(null)
    setBusyAction('apiKey')
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        groupName: apiKeyGroupName.trim() || undefined,
        note: apiKeyNote.trim() || undefined,
        groupNote: resolvePendingGroupNoteForName(apiKeyGroupName) || undefined,
        apiKey: apiKeyValue.trim(),
        localPrimaryLimit: normalizeNumberInput(apiKeyPrimaryLimit),
        localSecondaryLimit: normalizeNumberInput(apiKeySecondaryLimit),
        localLimitUnit: apiKeyLimitUnit.trim() || 'requests',
      })
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
      else if (status === 'pending') accumulator.pending += 1
      else accumulator.draft += 1
      return accumulator
    },
    { total: 0, draft: 0, pending: 0, completed: 0 },
  )

  return (
    <div className="grid gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div className="section-heading">
              <Button asChild variant="ghost" size="sm" className="mb-1 self-start px-0">
                <Link to="/account-pool/upstream-accounts">
                  <Icon icon="mdi:arrow-left" className="mr-2 h-4 w-4" aria-hidden />
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
              <Icon icon="mdi:shield-key-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>
                <p className="font-medium">{t('accountPool.upstreamAccounts.writesDisabledTitle')}</p>
                <p className="mt-1 text-sm text-warning/90">{t('accountPool.upstreamAccounts.writesDisabledBody')}</p>
              </div>
            </Alert>
          ) : null}

          {error || actionError ? (
            <Alert variant="error">
              <Icon icon="mdi:alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
              <div>{actionError ?? error}</div>
            </Alert>
          ) : null}

          {session ? (
            <Alert variant={session.status === 'completed' ? 'success' : session.status === 'pending' ? 'info' : 'warning'}>
              <Icon
                icon={session.status === 'completed' ? 'mdi:check-circle-outline' : 'mdi:link-variant-plus'}
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="space-y-1">
                <p className="font-medium">{t(`accountPool.upstreamAccounts.oauth.status.${session.status}`)}</p>
                <p className="text-sm opacity-90">{sessionHint ?? session.error ?? formatDateTime(session.expiresAt)}</p>
              </div>
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
                        <Icon icon="mdi:information-outline" className="h-4 w-4" aria-hidden />
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
                        <Icon icon="mdi:file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                    <Button type="button" variant="secondary" onClick={appendBatchRow} disabled={!writesEnabled} className="h-10 shrink-0 rounded-lg">
                      <Icon icon="mdi:playlist-plus" className="mr-2 h-4 w-4" aria-hidden />
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
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.displayName')}</span>
                    <Input
                      name="oauthDisplayName"
                      value={oauthDisplayName}
                      onChange={(event) => setOauthDisplayName(event.target.value)}
                    />
                  </label>
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
                        onValueChange={setOauthGroupName}
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
                        disabled={!writesEnabled || !normalizeGroupName(oauthGroupName)}
                      >
                        <Icon icon="mdi:file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                  </label>
                  <label className="field">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                    <textarea
                      className="min-h-28 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                      name="oauthNote"
                      value={oauthNote}
                      onChange={(event) => setOauthNote(event.target.value)}
                    />
                  </label>

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
                          disabled={busyAction === 'oauth-generate' || !writesEnabled}
                        >
                          {busyAction === 'oauth-generate' ? (
                            <Icon icon="mdi:loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                          ) : (
                            <Icon icon="mdi:link-variant-plus" className="mr-2 h-4 w-4" aria-hidden />
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
                              <Icon icon="mdi:content-copy" className="mr-2 h-4 w-4" aria-hidden />
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
                      disabled={!oauthSessionActive || !oauthCallbackUrl.trim() || busyAction === 'oauth-complete' || !writesEnabled}
                    >
                      {busyAction === 'oauth-complete' ? (
                        <Icon icon="mdi:loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                      ) : (
                        <Icon icon="mdi:check-decagram-outline" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.completeOauth')}
                    </Button>
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
                            const isCompleted = status === 'completed'
                            const isPending = status === 'pending'
                            const isBusy = row.busyAction != null
                            const rowLocked = isBusy || isCompleted
                            const authUrl = row.session?.authUrl ?? ''
                            return (
                              <tr
                                key={row.id}
                                data-testid={`batch-oauth-row-${row.id}`}
                                className="align-top border-b border-base-300/70 last:border-b-0"
                              >
                                <td className="px-3 py-4">
                                  <span className="inline-flex h-8 min-w-8 items-center justify-center rounded-full border border-base-300/80 px-2 text-sm font-semibold text-base-content/72">
                                    {index + 1}
                                  </span>
                                </td>
                                <td className="px-3 py-4">
                                  <div className="grid gap-3">
                                    <label className="field min-w-0 gap-2 whitespace-nowrap">
                                      <span className="field-label">{t('accountPool.upstreamAccounts.fields.displayName')}</span>
                                      <Input
                                        name={`batchOauthDisplayName-${row.id}`}
                                        value={row.displayName}
                                        disabled={rowLocked}
                                        className="min-w-0"
                                        onChange={(event) => handleBatchMetadataChange(row.id, 'displayName', event.target.value)}
                                      />
                                    </label>
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
                                          className="shrink-0 rounded-full"
                                          aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                          title={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                          onClick={() => openGroupNoteEditor(row.groupName)}
                                          disabled={!writesEnabled || !normalizeGroupName(row.groupName)}
                                        >
                                          <Icon icon="mdi:file-document-edit-outline" className="h-4 w-4" aria-hidden />
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
                                            disabled={isBusy || isCompleted || !writesEnabled}
                                          >
                                            {row.busyAction === 'generate' ? (
                                              <Spinner size="sm" />
                                            ) : (
                                              <Icon icon={isPending ? 'mdi:refresh' : 'mdi:link-variant-plus'} className="h-4 w-4" aria-hidden />
                                            )}
                                          </Button>
                                        </Tooltip>
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
                                                <Icon icon="mdi:content-copy" className="h-4 w-4" aria-hidden />
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
                                            t('accountPool.upstreamAccounts.groupNotes.actions.edit'),
                                            t('accountPool.upstreamAccounts.groupNotes.tooltip.body'),
                                          )}
                                        >
                                          <Button
                                            type="button"
                                            size="icon"
                                            variant={hasGroupNote(row.groupName) ? 'secondary' : 'ghost'}
                                            className="h-9 w-9 shrink-0 rounded-full"
                                            aria-label={t('accountPool.upstreamAccounts.groupNotes.actions.edit')}
                                            onClick={() => openGroupNoteEditor(row.groupName)}
                                            disabled={!writesEnabled || !normalizeGroupName(row.groupName)}
                                          >
                                            <Icon icon="mdi:file-document-edit-outline" className="h-4 w-4" aria-hidden />
                                          </Button>
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
                                            <Icon
                                              icon={row.noteExpanded ? 'mdi:chevron-up' : 'mdi:note-text-outline'}
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
                                            disabled={!writesEnabled || isBusy || isCompleted || !isPending || !row.callbackUrl.trim()}
                                          >
                                            {row.busyAction === 'complete' ? (
                                              <Spinner size="sm" />
                                            ) : (
                                              <Icon icon="mdi:check-bold" className="h-4 w-4" aria-hidden />
                                            )}
                                          </Button>
                                        </Tooltip>
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
                                            <Icon icon="mdi:delete-outline" className="h-4 w-4" aria-hidden />
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
                    <Input
                      name="apiKeyDisplayName"
                      value={apiKeyDisplayName}
                      onChange={(event) => setApiKeyDisplayName(event.target.value)}
                    />
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
                        <Icon icon="mdi:file-document-edit-outline" className="h-4 w-4" aria-hidden />
                      </Button>
                    </div>
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.apiKey')}</span>
                    <Input
                      name="apiKeyValue"
                      value={apiKeyValue}
                      onChange={(event) => setApiKeyValue(event.target.value)}
                    />
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
                  <div className="md:col-span-2 flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">{t('accountPool.upstreamAccounts.actions.cancel')}</Link>
                    </Button>
                    <Button type="button" onClick={() => void handleCreateApiKey()} disabled={busyAction === 'apiKey' || !writesEnabled}>
                      {busyAction === 'apiKey' ? (
                        <Icon icon="mdi:loading" className="mr-2 h-4 w-4 animate-spin" aria-hidden />
                      ) : (
                        <Icon icon="mdi:content-save-plus-outline" className="mr-2 h-4 w-4" aria-hidden />
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
    </div>
  )
}
