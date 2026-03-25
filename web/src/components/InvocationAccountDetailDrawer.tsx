import { useEffect, useId, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { AccountDetailDrawerShell } from './AccountDetailDrawerShell'
import { AppIcon } from './AppIcon'
import { MotherAccountBadge } from './MotherAccountToggle'
import { UpstreamAccountUsageCard } from './UpstreamAccountUsageCard'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import { SegmentedControl, SegmentedControlItem } from './ui/segmented-control'
import { useTranslation } from '../i18n'
import type { UpstreamAccountDetail, UpstreamAccountDuplicateInfo } from '../lib/api'
import { fetchUpstreamAccountDetail } from '../lib/api'

interface InvocationAccountDetailDrawerProps {
  open: boolean
  accountId: number | null
  accountLabel: string | null
  onClose: () => void
}

type InvocationAccountDetailTab = 'overview' | 'health'

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

function statusVariant(status: string): 'success' | 'warning' | 'error' | 'secondary' {
  if (status === 'active') return 'success'
  if (status === 'syncing') return 'warning'
  if (status === 'error' || status === 'needs_reauth') return 'error'
  return 'secondary'
}

function kindVariant(kind: string): 'secondary' | 'success' {
  return kind === 'oauth_codex' ? 'success' : 'secondary'
}

function DetailField({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric-cell">
      <p className="metric-label">{label}</p>
      <p className="mt-2 break-all text-sm text-base-content/80">{value || '—'}</p>
    </div>
  )
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

function formatDuplicateReasons(
  duplicateInfo: UpstreamAccountDuplicateInfo | null | undefined,
  t: (key: string) => string,
) {
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

export function InvocationAccountDetailDrawer({
  open,
  accountId,
  accountLabel,
  onClose,
}: InvocationAccountDetailDrawerProps) {
  const { t } = useTranslation()
  const requestSeqRef = useRef(0)
  const [detail, setDetail] = useState<UpstreamAccountDetail | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [detailTab, setDetailTab] = useState<InvocationAccountDetailTab>('overview')
  const titleId = useId()
  const tabsBaseId = useId()

  useEffect(() => {
    if (!open || accountId == null) {
      setDetail(null)
      setError(null)
      setIsLoading(false)
      return
    }

    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    setIsLoading(true)
    setError(null)
    void fetchUpstreamAccountDetail(accountId)
      .then((response) => {
        if (requestSeq !== requestSeqRef.current) return
        setDetail(response)
      })
      .catch((err) => {
        if (requestSeq !== requestSeqRef.current) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false)
        }
      })
  }, [accountId, open])

  useEffect(() => {
    if (!open) {
      setDetailTab('overview')
    }
  }, [open])

  useEffect(() => {
    setDetailTab('overview')
  }, [accountId])

  const title = detail?.displayName ?? accountLabel ?? t('table.accountDrawer.fallbackTitle')
  const statusLabel = detail ? t(`accountPool.upstreamAccounts.status.${detail.status}`) : null
  const kindLabel = detail
    ? detail.kind === 'oauth_codex'
      ? t('accountPool.upstreamAccounts.kind.oauth')
      : t('accountPool.upstreamAccounts.kind.apiKey')
    : null
  const tabIds = {
    overview: {
      tab: `${tabsBaseId}-overview-tab`,
      panel: `${tabsBaseId}-overview-panel`,
    },
    health: {
      tab: `${tabsBaseId}-health-tab`,
      panel: `${tabsBaseId}-health-panel`,
    },
  } as const

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t('table.accountDrawer.close')}
      onClose={onClose}
      shellClassName="max-w-[56rem]"
      header={
        <div className="space-y-4">
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              {statusLabel ? <Badge variant={statusVariant(detail?.status ?? '')}>{statusLabel}</Badge> : null}
              {kindLabel ? <Badge variant={kindVariant(detail?.kind ?? 'oauth_codex')}>{kindLabel}</Badge> : null}
              {detail?.planType ? <Badge variant="secondary">{detail.planType}</Badge> : null}
              {detail?.duplicateInfo ? (
                <Badge variant="warning">{t('accountPool.upstreamAccounts.duplicate.badge')}</Badge>
              ) : null}
            </div>
            <div className="section-heading">
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                {t('table.accountDrawer.subtitle')}
              </p>
              <div className="flex flex-wrap items-center gap-2">
                <h2 id={titleId} className="section-title">
                  {title}
                </h2>
                {detail?.isMother ? (
                  <MotherAccountBadge label={t('accountPool.upstreamAccounts.mother.badge')} />
                ) : null}
              </div>
              <p className="section-description">
                {detail?.email ?? detail?.maskedApiKey ?? t('accountPool.upstreamAccounts.identityUnavailable')}
              </p>
            </div>
          </div>
          {detail ? (
            <div className="flex flex-wrap items-center gap-2">
              <Button asChild variant="outline">
                <Link to="/account-pool/upstream-accounts" state={{ selectedAccountId: detail.id, openDetail: true }}>
                  <AppIcon name="arrow-right-bold" className="mr-2 h-4 w-4" aria-hidden />
                  {t('table.accountDrawer.openAccountPool')}
                </Link>
              </Button>
            </div>
          ) : null}
        </div>
      }
    >
      {isLoading ? (
        <AccountDetailSkeleton />
      ) : error ? (
        <div className="grid gap-4">
          <Alert variant="error">
            <AppIcon name="alert-circle-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
            <div>
              <p className="font-medium">{t('table.accountDrawer.errorTitle')}</p>
              <p className="mt-1 text-sm">{error}</p>
            </div>
          </Alert>
          {accountId != null ? (
            <Button asChild variant="outline" className="w-fit">
              <Link to="/account-pool/upstream-accounts" state={{ selectedAccountId: accountId, openDetail: true }}>
                <AppIcon name="arrow-right-bold" className="mr-2 h-4 w-4" aria-hidden />
                {t('table.accountDrawer.openAccountPool')}
              </Link>
            </Button>
          ) : null}
        </div>
      ) : !detail ? (
        <div className="flex min-h-[20rem] flex-col items-center justify-center rounded-[1.6rem] border border-dashed border-base-300/80 bg-base-100/45 px-6 text-center">
          <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-primary/10 text-primary">
            <AppIcon name="account-details-outline" className="h-7 w-7" aria-hidden />
          </div>
          <h3 className="text-lg font-semibold">{t('table.accountDrawer.emptyTitle')}</h3>
          <p className="mt-2 max-w-sm text-sm leading-6 text-base-content/65">
            {t('table.accountDrawer.emptyBody')}
          </p>
        </div>
      ) : (
        <div className="grid gap-5">
          <SegmentedControl
            className="self-start"
            role="tablist"
            aria-label={t('table.accountDrawer.subtitle')}
          >
            <SegmentedControlItem
              id={tabIds.overview.tab}
              active={detailTab === 'overview'}
              role="tab"
              aria-selected={detailTab === 'overview'}
              aria-controls={tabIds.overview.panel}
              aria-pressed={detailTab === 'overview'}
              onClick={() => setDetailTab('overview')}
            >
              {t('table.accountDrawer.tabs.overview')}
            </SegmentedControlItem>
            <SegmentedControlItem
              id={tabIds.health.tab}
              active={detailTab === 'health'}
              role="tab"
              aria-selected={detailTab === 'health'}
              aria-controls={tabIds.health.panel}
              aria-pressed={detailTab === 'health'}
              onClick={() => setDetailTab('health')}
            >
              {t('table.accountDrawer.tabs.health')}
            </SegmentedControlItem>
          </SegmentedControl>

          {detailTab === 'overview' ? (
            <div
              id={tabIds.overview.panel}
              role="tabpanel"
              aria-labelledby={tabIds.overview.tab}
              className="grid gap-5"
            >
              {detail.duplicateInfo ? (
                <Alert variant="warning">
                  <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                  <div>
                    <p className="font-medium">{t('accountPool.upstreamAccounts.duplicate.badge')}</p>
                    <p className="mt-1 text-sm text-warning/90">
                      {t('accountPool.upstreamAccounts.duplicate.warningBody', {
                        reasons: formatDuplicateReasons(detail.duplicateInfo, t),
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
                  value={
                    detail.isMother
                      ? t('accountPool.upstreamAccounts.mother.badge')
                      : t('accountPool.upstreamAccounts.mother.notMother')
                  }
                />
                <DetailField label={t('accountPool.upstreamAccounts.fields.email')} value={detail.email ?? detail.maskedApiKey ?? ''} />
                <DetailField label={t('accountPool.upstreamAccounts.fields.accountId')} value={detail.chatgptAccountId ?? detail.maskedApiKey ?? ''} />
                <DetailField label={t('accountPool.upstreamAccounts.fields.userId')} value={detail.chatgptUserId ?? ''} />
                <DetailField label={t('accountPool.upstreamAccounts.fields.lastSuccessSync')} value={formatDateTime(detail.lastSuccessfulSyncAt)} />
              </div>
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
            </div>
          ) : null}

          {detailTab === 'health' ? (
            <div
              id={tabIds.health.panel}
              role="tabpanel"
              aria-labelledby={tabIds.health.tab}
            >
              <Card className="border-base-300/80 bg-base-100/72">
                <CardHeader>
                  <CardTitle>{t('table.accountDrawer.healthTitle')}</CardTitle>
                  <CardDescription>{t('table.accountDrawer.healthDescription')}</CardDescription>
                </CardHeader>
                <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                  <DetailField label={t('accountPool.upstreamAccounts.fields.lastSyncedAt')} value={formatDateTime(detail.lastSyncedAt)} />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.lastRefreshedAt')} value={formatDateTime(detail.lastRefreshedAt)} />
                  <DetailField label={t('accountPool.upstreamAccounts.fields.tokenExpiresAt')} value={formatDateTime(detail.tokenExpiresAt)} />
                  <DetailField
                    label={t('accountPool.upstreamAccounts.fields.credits')}
                    value={
                      detail.credits?.balance
                        ? `${detail.credits.balance}`
                        : detail.credits?.unlimited
                          ? t('accountPool.upstreamAccounts.unlimited')
                          : t('accountPool.upstreamAccounts.unavailable')
                    }
                  />
                  <div className="md:col-span-2 xl:col-span-4 rounded-[1.2rem] border border-base-300/80 bg-base-100/75 p-4">
                    <p className="metric-label">{t('accountPool.upstreamAccounts.fields.lastError')}</p>
                    <p className="mt-2 text-sm leading-6 text-base-content/75">
                      {detail.lastError ?? t('accountPool.upstreamAccounts.noError')}
                    </p>
                    <p className="mt-2 text-xs text-base-content/55">{formatDateTime(detail.lastErrorAt)}</p>
                  </div>
                </CardContent>
              </Card>
            </div>
          ) : null}
        </div>
      )}
    </AccountDetailDrawerShell>
  )
}
