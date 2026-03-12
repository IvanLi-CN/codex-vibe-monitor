import { useEffect, useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import { Link, useLocation, useNavigate } from 'react-router-dom'
import { Alert } from '../../components/ui/alert'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { Input } from '../../components/ui/input'
import { Spinner } from '../../components/ui/spinner'
import { UpstreamAccountGroupCombobox } from '../../components/UpstreamAccountGroupCombobox'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import type { LoginSessionStatusResponse } from '../../lib/api'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

type CreateTab = 'oauth' | 'apiKey'

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

async function copyText(value: string) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value)
    return
  }

  const input = document.createElement('textarea')
  input.value = value
  input.setAttribute('readonly', '')
  input.style.position = 'absolute'
  input.style.left = '-9999px'
  document.body.appendChild(input)
  input.select()
  const copied = document.execCommand('copy')
  document.body.removeChild(input)
  if (!copied) {
    throw new Error('copy failed')
  }
}

export default function UpstreamAccountCreatePage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const location = useLocation()
  const {
    items,
    writesEnabled,
    isLoading,
    error,
    beginOauthLogin,
    completeOauthLogin,
    createApiKeyAccount,
  } = useUpstreamAccounts()

  const relinkAccountId = useMemo(() => parseAccountId(location.search), [location.search])
  const relinkSummary = useMemo(
    () => (relinkAccountId == null ? null : items.find((item) => item.id === relinkAccountId) ?? null),
    [items, relinkAccountId],
  )
  const isRelinking = relinkAccountId != null

  const [activeTab, setActiveTab] = useState<CreateTab>('oauth')
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

  const groupSuggestions = Array.from(
    new Set(
      items
        .map((item) => item.groupName?.trim())
        .filter((value): value is string => Boolean(value)),
    ),
  ).sort((left, right) => left.localeCompare(right))

  useEffect(() => {
    if (!isRelinking || !relinkSummary) return
    setActiveTab('oauth')
    setOauthDisplayName((current) => current || relinkSummary.displayName)
    setOauthGroupName((current) => current || relinkSummary.groupName || '')
  }, [isRelinking, relinkSummary])

  const handleGenerateOauthUrl = async () => {
    setActionError(null)
    setSessionHint(null)
    setBusyAction('oauth-generate')
    try {
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupName.trim() || undefined,
        note: oauthNote.trim() || undefined,
        accountId: relinkAccountId ?? undefined,
      })
      setSession(response)
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
    try {
      await copyText(session.authUrl)
      setSessionHint(t('accountPool.upstreamAccounts.oauth.copied'))
    } catch (err) {
      setActionError(
        err instanceof Error && err.message !== 'copy failed'
          ? err.message
          : t('accountPool.upstreamAccounts.oauth.copyFailed'),
      )
    }
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

  const handleCreateApiKey = async () => {
    setActionError(null)
    setBusyAction('apiKey')
    try {
      const response = await createApiKeyAccount({
        displayName: apiKeyDisplayName.trim(),
        groupName: apiKeyGroupName.trim() || undefined,
        note: apiKeyNote.trim() || undefined,
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
              {(['oauth', 'apiKey'] as const).map((tab) => (
                <button
                  key={tab}
                  type="button"
                  role="tab"
                  aria-selected={activeTab === tab}
                  className="segment-button"
                  data-active={activeTab === tab}
                  onClick={() => setActiveTab(tab)}
                >
                  {tab === 'oauth'
                    ? t('accountPool.upstreamAccounts.createPage.tabs.oauth')
                    : t('accountPool.upstreamAccounts.createPage.tabs.apiKey')}
                </button>
              ))}
            </div>
          ) : null}

          <Card className="border-base-300/80 bg-base-100/72">
            <CardHeader>
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
                    <UpstreamAccountGroupCombobox
                      name="oauthGroupName"
                      value={oauthGroupName}
                      suggestions={groupSuggestions}
                      placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                      searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                      emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                      createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                      onValueChange={setOauthGroupName}
                    />
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
                    <div className="space-y-1">
                      <h3 className="text-sm font-semibold text-base-content">
                        {t('accountPool.upstreamAccounts.oauth.manualFlowTitle')}
                      </h3>
                      <p className="text-sm text-base-content/70">
                        {t('accountPool.upstreamAccounts.oauth.manualFlowDescription')}
                      </p>
                    </div>

                    <div className="mt-4 grid gap-4">
                      <div className="rounded-xl border border-dashed border-base-300/90 bg-base-100/55 p-4">
                        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                          <div className="space-y-1">
                            <p className="text-sm font-medium text-base-content">
                              {t('accountPool.upstreamAccounts.oauth.authUrlLabel')}
                            </p>
                            <p className="text-xs text-base-content/65">
                              {t('accountPool.upstreamAccounts.oauth.authUrlDescription')}
                            </p>
                          </div>
                          <Button
                            type="button"
                            variant="secondary"
                            onClick={() => void handleCopyOauthUrl()}
                            disabled={!oauthSessionActive || !session?.authUrl}
                          >
                            <Icon icon="mdi:content-copy" className="mr-2 h-4 w-4" aria-hidden />
                            {t('accountPool.upstreamAccounts.actions.copyOauthUrl')}
                          </Button>
                        </div>
                        <textarea
                          readOnly
                          value={session?.authUrl ?? ''}
                          placeholder={t('accountPool.upstreamAccounts.oauth.authUrlPlaceholder')}
                          className="mt-3 min-h-28 w-full rounded-xl border border-base-300 bg-base-100 px-3 py-2 font-mono text-xs text-base-content/80 shadow-sm focus-visible:outline-none"
                        />
                      </div>

                      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                        <label className="field">
                          <span className="field-label">{t('accountPool.upstreamAccounts.oauth.redirectUriLabel')}</span>
                          <Input readOnly value={session?.redirectUri ?? ''} placeholder={t('accountPool.upstreamAccounts.oauth.redirectUriPlaceholder')} />
                          <span className="text-xs text-base-content/60">
                            {t('accountPool.upstreamAccounts.oauth.redirectUriDescription')}
                          </span>
                        </label>
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
                      variant="secondary"
                      onClick={() => void handleGenerateOauthUrl()}
                      disabled={busyAction === 'oauth-generate' || !writesEnabled}
                    >
                      {busyAction === 'oauth-generate' ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <Icon icon="mdi:link-variant-plus" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {session?.status === 'pending'
                        ? t('accountPool.upstreamAccounts.actions.regenerateOauthUrl')
                        : t('accountPool.upstreamAccounts.actions.generateOauthUrl')}
                    </Button>
                    <Button
                      type="button"
                      onClick={() => void handleCompleteOauth()}
                      disabled={!oauthSessionActive || !oauthCallbackUrl.trim() || busyAction === 'oauth-complete' || !writesEnabled}
                    >
                      {busyAction === 'oauth-complete' ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <Icon icon="mdi:check-decagram-outline" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.completeOauth')}
                    </Button>
                  </div>
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
                    <UpstreamAccountGroupCombobox
                      name="apiKeyGroupName"
                      value={apiKeyGroupName}
                      suggestions={groupSuggestions}
                      placeholder={t('accountPool.upstreamAccounts.fields.groupNamePlaceholder')}
                      searchPlaceholder={t('accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder')}
                      emptyLabel={t('accountPool.upstreamAccounts.fields.groupNameEmpty')}
                      createLabel={(value) => t('accountPool.upstreamAccounts.fields.groupNameUseValue', { value })}
                      onValueChange={setApiKeyGroupName}
                    />
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
                        <Spinner size="sm" className="mr-2" />
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
    </div>
  )
}
