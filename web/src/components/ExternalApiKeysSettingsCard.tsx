import { useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import { useTranslation } from '../i18n'
import { useExternalApiKeys } from '../hooks/useExternalApiKeys'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog'
import { Input } from './ui/input'
import { Spinner } from './ui/spinner'
import type { ExternalApiKeySummary } from '../lib/api'

function formatDateTime(value?: string) {
  if (!value) return '—'
  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) return value
  return parsed.toLocaleString()
}

function statusVariant(status: string): 'success' | 'warning' | 'secondary' {
  if (status === 'active') return 'success'
  if (status === 'disabled') return 'warning'
  return 'secondary'
}

function statusLabel(status: string, t: (key: string) => string) {
  switch (status) {
    case 'active':
      return t('settings.externalApiKeys.status.active')
    case 'disabled':
      return t('settings.externalApiKeys.status.disabled')
    default:
      return status
  }
}

export function ExternalApiKeysSettingsCard() {
  const { t } = useTranslation()
  const {
    items,
    activeCount,
    isLoading,
    isMutating,
    error,
    revealedSecret,
    createKey,
    rotateKey,
    disableKey,
    clearRevealedSecret,
  } = useExternalApiKeys()

  const [createOpen, setCreateOpen] = useState(false)
  const [createName, setCreateName] = useState('')
  const [createError, setCreateError] = useState<string | null>(null)
  const [rotateTarget, setRotateTarget] = useState<ExternalApiKeySummary | null>(null)
  const [disableTarget, setDisableTarget] = useState<ExternalApiKeySummary | null>(null)

  const summaryText = useMemo(
    () => t('settings.externalApiKeys.summary', { count: activeCount, total: items.length }),
    [activeCount, items.length, t],
  )

  async function handleCreateSubmit() {
    const normalized = createName.trim()
    if (!normalized) {
      setCreateError(t('settings.externalApiKeys.validation.nameRequired'))
      return
    }
    try {
      await createKey(normalized)
      setCreateName('')
      setCreateError(null)
      setCreateOpen(false)
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : String(err))
    }
  }

  async function handleRotateConfirm() {
    if (!rotateTarget) return
    try {
      await rotateKey(rotateTarget.id)
      setRotateTarget(null)
    } catch {
      // hook-level error already surfaced
    }
  }

  async function handleDisableConfirm() {
    if (!disableTarget) return
    try {
      await disableKey(disableTarget.id)
      setDisableTarget(null)
    } catch {
      // hook-level error already surfaced
    }
  }

  return (
    <>
      <Card className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
        <CardHeader className="flex-row items-start justify-between gap-3 space-y-0 border-b border-base-300/70 pb-4">
          <div className="space-y-1.5">
            <CardTitle>{t('settings.externalApiKeys.title')}</CardTitle>
            <div className="space-y-1">
              <CardDescription>{t('settings.externalApiKeys.description')}</CardDescription>
              <p className="text-xs text-base-content/65">{summaryText}</p>
            </div>
          </div>
          <Button
            type="button"
            size="sm"
            className="h-9 gap-1.5 px-3.5"
            onClick={() => {
              setCreateError(null)
              setCreateOpen(true)
            }}
          >
            <AppIcon name="plus" className="h-[18px] w-[18px]" aria-hidden />
            {t('settings.externalApiKeys.create')}
          </Button>
        </CardHeader>

        <CardContent className="space-y-4 pt-4">
          {revealedSecret && (
            <Alert variant="success" className="flex-col gap-3" data-testid="external-api-key-secret-alert">
              <div className="space-y-1">
                <div className="font-medium">
                  {revealedSecret.action === 'create'
                    ? t('settings.externalApiKeys.secret.createdTitle')
                    : t('settings.externalApiKeys.secret.rotatedTitle')}
                </div>
                <div className="text-xs leading-6">
                  {t('settings.externalApiKeys.secret.description')}
                </div>
              </div>
              <div className="rounded-lg border border-success/30 bg-base-100/85 px-3 py-2 font-mono text-xs text-base-content">
                {revealedSecret.secret}
              </div>
              <div className="flex justify-end">
                <Button type="button" variant="ghost" size="sm" onClick={clearRevealedSecret}>
                  {t('settings.externalApiKeys.secret.dismiss')}
                </Button>
              </div>
            </Alert>
          )}

          {error && (
            <Alert variant="error" data-testid="external-api-key-error">
              {t('settings.externalApiKeys.error', { error })}
            </Alert>
          )}

          {isLoading ? (
            <div className="flex items-center gap-2 rounded-xl border border-base-300/80 bg-base-100/72 px-4 py-6 text-sm text-base-content/72">
              <Spinner size="sm" />
              {t('settings.externalApiKeys.loading')}
            </div>
          ) : items.length === 0 ? (
            <div className="rounded-xl border border-dashed border-base-300/80 bg-base-100/72 px-4 py-8 text-center text-sm text-base-content/65">
              {t('settings.externalApiKeys.empty')}
            </div>
          ) : (
            <div className="overflow-x-auto rounded-xl border border-base-300/80 bg-base-100/72">
              <table className="w-full min-w-[44rem] table-fixed text-sm">
                <thead className="bg-base-200/70 text-[11px] uppercase tracking-[0.08em] text-base-content/65">
                  <tr>
                    <th className="px-4 py-3 text-left font-semibold">{t('settings.externalApiKeys.columns.name')}</th>
                    <th className="px-4 py-3 text-left font-semibold">{t('settings.externalApiKeys.columns.prefix')}</th>
                    <th className="px-4 py-3 text-left font-semibold">{t('settings.externalApiKeys.columns.status')}</th>
                    <th className="px-4 py-3 text-left font-semibold">{t('settings.externalApiKeys.columns.lastUsedAt')}</th>
                    <th className="px-4 py-3 text-left font-semibold">{t('settings.externalApiKeys.columns.createdAt')}</th>
                    <th className="px-4 py-3 text-right font-semibold">{t('settings.externalApiKeys.columns.actions')}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-base-300/65">
                  {items.map((item, index) => (
                    <tr
                      key={item.id}
                      className={
                        index % 2 === 0
                          ? 'bg-base-100/38 transition-colors hover:bg-primary/6'
                          : 'bg-base-200/22 transition-colors hover:bg-primary/6'
                      }
                    >
                      <td className="px-4 py-3 align-middle">
                        <div className="font-medium text-base-content">{item.name}</div>
                      </td>
                      <td className="px-4 py-3 align-middle">
                        <code className="rounded-md border border-base-300/70 bg-base-100/70 px-2 py-1 text-xs">
                          {item.prefix}
                        </code>
                      </td>
                      <td className="px-4 py-3 align-middle">
                        <Badge variant={statusVariant(item.status)}>
                          {statusLabel(item.status, t)}
                        </Badge>
                      </td>
                      <td className="px-4 py-3 align-middle text-base-content/72">
                        {formatDateTime(item.lastUsedAt)}
                      </td>
                      <td className="px-4 py-3 align-middle text-base-content/72">
                        {formatDateTime(item.createdAt)}
                      </td>
                      <td className="px-4 py-3 align-middle">
                        <div className="flex justify-end gap-2">
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            disabled={isMutating}
                            onClick={() => setRotateTarget(item)}
                          >
                            {t('settings.externalApiKeys.rotate')}
                          </Button>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="text-error hover:bg-error/10 hover:text-error"
                            disabled={isMutating || item.status === 'disabled'}
                            onClick={() => setDisableTarget(item)}
                          >
                            {t('settings.externalApiKeys.disable')}
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('settings.externalApiKeys.createDialog.title')}</DialogTitle>
            <DialogDescription>{t('settings.externalApiKeys.createDialog.description')}</DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <label htmlFor="external-api-key-name" className="block text-sm font-medium text-base-content/78">
              {t('settings.externalApiKeys.createDialog.nameLabel')}
            </label>
            <Input
              id="external-api-key-name"
              value={createName}
              placeholder={t('settings.externalApiKeys.createDialog.namePlaceholder')}
              onChange={(event) => {
                setCreateName(event.target.value)
                if (createError) setCreateError(null)
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault()
                  void handleCreateSubmit()
                }
              }}
            />
            {createError && <p className="text-sm text-error">{createError}</p>}
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setCreateOpen(false)}>
              {t('settings.externalApiKeys.cancel')}
            </Button>
            <Button type="button" disabled={isMutating} onClick={() => void handleCreateSubmit()}>
              {isMutating ? t('settings.saving') : t('settings.externalApiKeys.createDialog.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={rotateTarget != null} onOpenChange={(open) => !open && setRotateTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('settings.externalApiKeys.rotateDialog.title')}</DialogTitle>
            <DialogDescription>
              {t('settings.externalApiKeys.rotateDialog.description', {
                name: rotateTarget?.name ?? '—',
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setRotateTarget(null)}>
              {t('settings.externalApiKeys.cancel')}
            </Button>
            <Button type="button" disabled={isMutating || !rotateTarget} onClick={() => void handleRotateConfirm()}>
              {isMutating ? t('settings.saving') : t('settings.externalApiKeys.rotateDialog.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={disableTarget != null} onOpenChange={(open) => !open && setDisableTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t('settings.externalApiKeys.disableDialog.title')}</DialogTitle>
            <DialogDescription>
              {t('settings.externalApiKeys.disableDialog.description', {
                name: disableTarget?.name ?? '—',
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setDisableTarget(null)}>
              {t('settings.externalApiKeys.cancel')}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={isMutating || !disableTarget}
              onClick={() => void handleDisableConfirm()}
            >
              {isMutating ? t('settings.saving') : t('settings.externalApiKeys.disableDialog.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
