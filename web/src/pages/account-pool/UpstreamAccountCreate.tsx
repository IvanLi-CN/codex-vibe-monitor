import { useEffect, useState } from 'react'
import { Icon } from '@iconify/react'
import { Link, useNavigate } from 'react-router-dom'
import { Alert } from '../../components/ui/alert'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { Input } from '../../components/ui/input'
import { Spinner } from '../../components/ui/spinner'
import { useUpstreamAccounts } from '../../hooks/useUpstreamAccounts'
import type { LoginSessionStatusResponse } from '../../lib/api'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

type CreateTab = 'oauth' | 'apiKey'

const LOGIN_POLL_INTERVAL_MS = 1500
const POPUP_CLOSE_CHECK_INTERVAL_MS = 1000

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

export default function UpstreamAccountCreatePage() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const {
    writesEnabled,
    isLoading,
    error,
    beginOauthLogin,
    getLoginSession,
    createApiKeyAccount,
  } = useUpstreamAccounts()

  const [activeTab, setActiveTab] = useState<CreateTab>('oauth')
  const [oauthDisplayName, setOauthDisplayName] = useState('')
  const [oauthGroupName, setOauthGroupName] = useState('')
  const [oauthNote, setOauthNote] = useState('')
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
  const [popupWindow, setPopupWindow] = useState<Window | null>(null)

  useEffect(() => {
    const currentSession = session
    if (!currentSession || currentSession.status !== 'pending') return undefined

    const pollTimer = window.setInterval(() => {
      void (async () => {
        try {
          const next = await getLoginSession(currentSession.loginId)
          setSession(next)
          if (next.status === 'completed' && next.accountId != null) {
            if (popupWindow && !popupWindow.closed) {
              popupWindow.close()
            }
            navigate('/account-pool/upstream-accounts', {
              state: {
                selectedAccountId: next.accountId,
                openDetail: true,
              },
            })
            return
          }
          if (next.status === 'failed' || next.status === 'expired') {
            setSessionHint(next.error ?? t('accountPool.upstreamAccounts.oauth.failed'))
          }
        } catch (err) {
          setActionError(err instanceof Error ? err.message : String(err))
        }
      })()
    }, LOGIN_POLL_INTERVAL_MS)

    const popupTimer = window.setInterval(() => {
      if (popupWindow && popupWindow.closed) {
        setSessionHint(t('accountPool.upstreamAccounts.oauth.popupClosed'))
      }
    }, POPUP_CLOSE_CHECK_INTERVAL_MS)

    return () => {
      window.clearInterval(pollTimer)
      window.clearInterval(popupTimer)
    }
  }, [getLoginSession, navigate, popupWindow, session, t])

  const handleOauthLogin = async () => {
    setActionError(null)
    setSessionHint(null)
    setBusyAction('oauth')
    try {
      const response = await beginOauthLogin({
        displayName: oauthDisplayName.trim() || undefined,
        groupName: oauthGroupName.trim() || undefined,
        note: oauthNote.trim() || undefined,
      })
      setSession(response)
      const popup = response.authUrl
        ? window.open(response.authUrl, 'codex-upstream-login', 'popup=yes,width=560,height=760')
        : null
      setPopupWindow(popup)
      if (!popup && response.authUrl) {
        window.open(response.authUrl, '_blank', 'noopener,noreferrer')
        setSessionHint(t('accountPool.upstreamAccounts.oauth.popupFallback'))
      }
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
              <h2 className="section-title">{t('accountPool.upstreamAccounts.createPage.title')}</h2>
              <p className="section-description">{t('accountPool.upstreamAccounts.createPage.description')}</p>
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
                icon={session.status === 'completed' ? 'mdi:check-circle-outline' : 'mdi:account-clock-outline'}
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div className="space-y-1">
                <p className="font-medium">{t(`accountPool.upstreamAccounts.oauth.status.${session.status}`)}</p>
                <p className="text-sm opacity-90">{sessionHint ?? session.error ?? formatDateTime(session.expiresAt)}</p>
                {session.authUrl ? (
                  <a className="app-link text-xs" href={session.authUrl} target="_blank" rel="noreferrer">
                    {t('accountPool.upstreamAccounts.oauth.openAgain')}
                  </a>
                ) : null}
              </div>
            </Alert>
          ) : null}

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
                    <Input
                      name="oauthGroupName"
                      value={oauthGroupName}
                      onChange={(event) => setOauthGroupName(event.target.value)}
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
                  <div className="flex flex-wrap justify-end gap-2">
                    <Button asChild type="button" variant="ghost">
                      <Link to="/account-pool/upstream-accounts">{t('accountPool.upstreamAccounts.actions.cancel')}</Link>
                    </Button>
                    <Button type="button" onClick={() => void handleOauthLogin()} disabled={busyAction === 'oauth' || !writesEnabled}>
                      {busyAction === 'oauth' ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <Icon icon="mdi:login-variant" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t('accountPool.upstreamAccounts.actions.startOauth')}
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
                    <Input
                      name="apiKeyGroupName"
                      value={apiKeyGroupName}
                      onChange={(event) => setApiKeyGroupName(event.target.value)}
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
