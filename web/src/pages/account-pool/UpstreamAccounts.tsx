import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Icon } from '@iconify/react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { Alert } from '../../components/ui/alert'
import { Badge } from '../../components/ui/badge'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { Input } from '../../components/ui/input'
import { Spinner } from '../../components/ui/spinner'
import { Switch } from '../../components/ui/switch'
import { UpstreamAccountGroupCombobox } from '../../components/UpstreamAccountGroupCombobox'
import { UpstreamAccountUsageCard } from '../../components/UpstreamAccountUsageCard'
import { UpstreamAccountsTable } from '../../components/UpstreamAccountsTable'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import type { UpstreamAccountDetail, UpstreamAccountSummary } from '../../lib/api'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

type AccountDraft = {
  displayName: string
  groupName: string
  note: string
  localPrimaryLimit: string
  localSecondaryLimit: string
  localLimitUnit: string
  apiKey: string
}

type UpstreamAccountsLocationState = {
  selectedAccountId?: number
  openDetail?: boolean
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

function buildDraft(detail: UpstreamAccountDetail | null): AccountDraft {
  return {
    displayName: detail?.displayName ?? '',
    groupName: detail?.groupName ?? '',
    note: detail?.note ?? '',
    localPrimaryLimit:
      detail?.localLimits?.primaryLimit == null ? '' : String(detail.localLimits.primaryLimit),
    localSecondaryLimit:
      detail?.localLimits?.secondaryLimit == null ? '' : String(detail.localLimits.secondaryLimit),
    localLimitUnit: detail?.localLimits?.limitUnit ?? 'requests',
    apiKey: '',
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


function poolCardMetric(value: number, label: string, icon: string, accent: string) {
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
  onClose,
  children,
}: {
  open: boolean
  title: string
  subtitle?: string
  closeLabel: string
  onClose: () => void
  children: ReactNode
}) {
  const closeButtonRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    if (!open || typeof document === 'undefined') return undefined

    const previousOverflow = document.body.style.overflow
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose()
      }
    }

    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', handleKeyDown)
    const focusTimer = window.setTimeout(() => closeButtonRef.current?.focus(), 0)

    return () => {
      window.clearTimeout(focusTimer)
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [onClose, open])

  if (!open || typeof document === 'undefined') return null

  return createPortal(
    <div className="fixed inset-0 z-[70]">
      <div
        aria-hidden="true"
        className="absolute inset-0 bg-neutral/50 backdrop-blur-sm"
        onClick={onClose}
      />
      <div className="absolute inset-y-0 right-0 flex w-full justify-end pl-4 sm:pl-8">
        <section
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
              <Button ref={closeButtonRef} type="button" variant="ghost" size="icon" onClick={onClose}>
                <Icon icon="mdi:close" className="h-5 w-5" aria-hidden />
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

export default function UpstreamAccountsPage() {
  const { t } = useTranslation()
  const location = useLocation()
  const navigate = useNavigate()
  const {
    items,
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
  } = useUpstreamAccounts()

  const [draft, setDraft] = useState<AccountDraft>(buildDraft(null))
  const [actionError, setActionError] = useState<string | null>(null)
  const [busyAction, setBusyAction] = useState<string | null>(null)
  const [isDetailDrawerOpen, setIsDetailDrawerOpen] = useState(false)
  const [groupFilterQuery, setGroupFilterQuery] = useState('')

  useEffect(() => {
    setDraft(buildDraft(detail))
  }, [detail])

  useEffect(() => {
    if (!selectedSummary && !detail) {
      setIsDetailDrawerOpen(false)
    }
  }, [detail, selectedSummary])

  useEffect(() => {
    const state = location.state as UpstreamAccountsLocationState | null
    if (!state?.selectedAccountId) return

    selectAccount(state.selectedAccountId)
    setIsDetailDrawerOpen(Boolean(state.openDetail))
    navigate(location.pathname, { replace: true, state: null })
  }, [location.pathname, location.state, navigate, selectAccount])

  const metrics = useMemo(() => {
    const oauthCount = items.filter((item) => item.kind === 'oauth_codex').length
    const apiKeyCount = items.filter((item) => item.kind === 'api_key_codex').length
    const needsReauthCount = items.filter((item) => item.status === 'needs_reauth').length
    const syncingCount = items.filter((item) => item.status === 'syncing').length
    return [
      poolCardMetric(items.length, t('accountPool.upstreamAccounts.metrics.total'), 'mdi:database-outline', 'text-primary'),
      poolCardMetric(oauthCount, t('accountPool.upstreamAccounts.metrics.oauth'), 'mdi:badge-account-horizontal-outline', 'text-success'),
      poolCardMetric(apiKeyCount, t('accountPool.upstreamAccounts.metrics.apiKey'), 'mdi:key-outline', 'text-info'),
      poolCardMetric(
        needsReauthCount + syncingCount,
        t('accountPool.upstreamAccounts.metrics.attention'),
        'mdi:alert-decagram-outline',
        'text-warning',
      ),
    ]
  }, [items, t])

  const availableGroups = useMemo(() => {
    const values = new Set<string>()
    let hasUngrouped = false
    for (const item of items) {
      const groupName = item.groupName?.trim()
      if (groupName) {
        values.add(groupName)
      } else {
        hasUngrouped = true
      }
    }
    return {
      names: Array.from(values).sort((left, right) => left.localeCompare(right)),
      hasUngrouped,
    }
  }, [items])

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

  const selected = detail ?? selectedSummary
  const selectedVisible = filteredItems.some((item) => item.id === selectedId)
  const accountStatusLabel = (status: string) => t(`accountPool.upstreamAccounts.status.${status}`)
  const accountKindLabel = (kind: string) =>
    kind === 'oauth_codex'
      ? t('accountPool.upstreamAccounts.kind.oauth')
      : t('accountPool.upstreamAccounts.kind.apiKey')
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

  const handleSave = async (source: UpstreamAccountDetail) => {
    setActionError(null)
    setBusyAction('save')
    try {
      await saveAccount(source.id, {
        displayName: draft.displayName.trim() || undefined,
        groupName: draft.groupName.trim(),
        note: draft.note.trim() || undefined,
        apiKey: source.kind === 'api_key_codex' && draft.apiKey.trim() ? draft.apiKey.trim() : undefined,
        localPrimaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localPrimaryLimit) : undefined,
        localSecondaryLimit: source.kind === 'api_key_codex' ? normalizeNumberInput(draft.localSecondaryLimit) : undefined,
        localLimitUnit: source.kind === 'api_key_codex' ? draft.localLimitUnit.trim() || undefined : undefined,
      })
      setDraft((current) => ({ ...current, apiKey: '' }))
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleSync = async (source: UpstreamAccountSummary) => {
    setActionError(null)
    setBusyAction('sync')
    try {
      await runSync(source.id)
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleToggleEnabled = async (source: UpstreamAccountSummary, enabled: boolean) => {
    setActionError(null)
    setBusyAction('toggle')
    try {
      await saveAccount(source.id, { enabled })
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyAction(null)
    }
  }

  const handleDelete = async (source: UpstreamAccountSummary) => {
    if (!window.confirm(t('accountPool.upstreamAccounts.deleteConfirm', { name: source.displayName }))) {
      return
    }
    setActionError(null)
    setBusyAction('delete')
    try {
      await removeAccount(source.id)
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
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
                  <Icon icon="mdi:refresh" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.upstreamAccounts.actions.refresh')}
                </Button>
                {writesEnabled ? (
                  <>
                    <Button asChild>
                      <Link to="/account-pool/upstream-accounts/new?mode=batchOauth">
                        <Icon icon="mdi:table-plus" className="mr-2 h-4 w-4" aria-hidden />
                        {t('accountPool.upstreamAccounts.actions.addBatchOauth')}
                      </Link>
                    </Button>
                    <Button asChild variant="secondary">
                      <Link to="/account-pool/upstream-accounts/new">
                        <Icon icon="mdi:plus-circle-outline" className="mr-2 h-4 w-4" aria-hidden />
                        {t('accountPool.upstreamAccounts.actions.addAccount')}
                      </Link>
                    </Button>
                  </>
                ) : (
                  <>
                    <Button type="button" disabled>
                      <Icon icon="mdi:table-plus" className="mr-2 h-4 w-4" aria-hidden />
                      {t('accountPool.upstreamAccounts.actions.addBatchOauth')}
                    </Button>
                    <Button type="button" variant="secondary" disabled>
                      <Icon icon="mdi:plus-circle-outline" className="mr-2 h-4 w-4" aria-hidden />
                      {t('accountPool.upstreamAccounts.actions.addAccount')}
                    </Button>
                  </>
                )}
              </div>
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

            {actionError ? (
              <Alert variant="error">
                <Icon icon="mdi:alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div>{actionError}</div>
              </Alert>
            ) : null}

            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              {metrics.map((metric) => (
                <Card key={metric.label} className="border-base-300/80 bg-base-100/72">
                  <CardContent className="flex items-center gap-4 p-5">
                    <div className={cn('flex h-12 w-12 items-center justify-center rounded-2xl bg-base-200/70', metric.accent)}>
                      <Icon icon={metric.icon} className="h-6 w-6" aria-hidden />
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

        <Card className="border-base-300/80 bg-base-100/72">
          <CardHeader>
            <CardTitle>{t('accountPool.upstreamAccounts.limitLegendTitle')}</CardTitle>
            <CardDescription>{t('accountPool.upstreamAccounts.limitLegendDescription')}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-base-content/65">
            <div className="rounded-2xl border border-base-300/80 bg-base-100/75 p-3">
              <p className="font-semibold text-base-content">{t('accountPool.upstreamAccounts.primaryWindowLabel')}</p>
              <p className="mt-1">{t('accountPool.upstreamAccounts.primaryWindowDescription')}</p>
            </div>
            <div className="rounded-2xl border border-base-300/80 bg-base-100/75 p-3">
              <p className="font-semibold text-base-content">{t('accountPool.upstreamAccounts.secondaryWindowLabel')}</p>
              <p className="mt-1">{t('accountPool.upstreamAccounts.secondaryWindowDescription')}</p>
            </div>
          </CardContent>
        </Card>
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
                    <Icon icon="mdi:account-details-outline" className="mr-2 h-4 w-4" aria-hidden />
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
                status: accountStatusLabel,
              }}
            />
          </div>
        </div>
      </section>

      <AccountDetailDrawer
        open={Boolean(selected && isDetailDrawerOpen)}
        title={selected?.displayName ?? t('accountPool.upstreamAccounts.detailTitle')}
        subtitle={t('accountPool.upstreamAccounts.detailTitle')}
        closeLabel={t('accountPool.upstreamAccounts.actions.closeDetails')}
        onClose={handleCloseDetailDrawer}
      >
        {!selected ? (
          <div className="flex min-h-[20rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 text-center">
            <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
              <Icon icon="mdi:account-details-outline" className="h-7 w-7" aria-hidden />
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
                  <Badge variant={statusVariant(selected.status)}>{accountStatusLabel(selected.status)}</Badge>
                  <Badge variant={kindVariant(selected.kind)}>{accountKindLabel(selected.kind)}</Badge>
                  {selected.planType ? <Badge variant="secondary">{selected.planType}</Badge> : null}
                  {selected.kind === 'api_key_codex' ? (
                    <Badge variant="secondary">{t('accountPool.upstreamAccounts.apiKey.localPlaceholder')}</Badge>
                  ) : null}
                </div>
                <div className="section-heading">
                  <h3 className="section-title">{selected.displayName}</h3>
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
                  {busyAction === 'sync' ? <Spinner size="sm" className="mr-2" /> : <Icon icon="mdi:refresh-circle" className="mr-2 h-4 w-4" aria-hidden />}
                  {t('accountPool.upstreamAccounts.actions.syncNow')}
                </Button>
                {selected.kind === 'oauth_codex' ? (
                  <Button type="button" variant="outline" onClick={() => void handleOauthLogin(selected.id)} disabled={busyAction === 'relogin' || !writesEnabled}>
                    {busyAction === 'relogin' ? <Spinner size="sm" className="mr-2" /> : <Icon icon="mdi:login-variant" className="mr-2 h-4 w-4" aria-hidden />}
                    {t('accountPool.upstreamAccounts.actions.relogin')}
                  </Button>
                ) : null}
                <Button type="button" variant="destructive" onClick={() => void handleDelete(selected)} disabled={busyAction === 'delete' || !writesEnabled}>
                  {busyAction === 'delete' ? <Spinner size="sm" className="mr-2" /> : <Icon icon="mdi:trash-can-outline" className="mr-2 h-4 w-4" aria-hidden />}
                  {t('accountPool.upstreamAccounts.actions.delete')}
                </Button>
              </div>
            </div>

            {detail ? (
              <div className="grid gap-5">
                <div className="metric-grid">
                  <DetailField label={t('accountPool.upstreamAccounts.fields.groupName')} value={detail.groupName ?? ''} />
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
                    <Input name="detailDisplayName" value={draft.displayName} onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))} />
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.groupName')}</span>
                    <UpstreamAccountGroupCombobox
                      name="detailGroupName"
                      value={draft.groupName}
                      suggestions={availableGroups.names}
                      placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                      searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                      emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                      createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                      onValueChange={(value) => setDraft((current) => ({ ...current, groupName: value }))}
                    />
                  </label>
                  <label className="field md:col-span-2">
                    <span className="field-label">{t('accountPool.upstreamAccounts.fields.note')}</span>
                      <textarea
                        className="min-h-24 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                        name="detailNote"
                        value={draft.note}
                        onChange={(event) => setDraft((current) => ({ ...current, note: event.target.value }))}
                      />
                    </label>
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
                      <Button type="button" onClick={() => void handleSave(detail)} disabled={busyAction === 'save' || !writesEnabled}>
                        {busyAction === 'save' ? <Spinner size="sm" className="mr-2" /> : <Icon icon="mdi:content-save-outline" className="mr-2 h-4 w-4" aria-hidden />}
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
                      <p className="metric-label">{t('accountPool.upstreamAccounts.fields.lastError')}</p>
                      <p className="mt-2 text-sm leading-6 text-base-content/75">{detail.lastError ?? t('accountPool.upstreamAccounts.noError')}</p>
                      <p className="mt-2 text-xs text-base-content/55">{formatDateTime(detail.lastErrorAt)}</p>
                    </div>
                  </CardContent>
                </Card>
              </div>
            ) : null}
          </>
        )}
      </AccountDetailDrawer>

      {error ? (
        <Alert variant="warning">
          <Icon icon="mdi:information-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
          <div>{error}</div>
        </Alert>
      ) : null}
    </div>
  )
}
