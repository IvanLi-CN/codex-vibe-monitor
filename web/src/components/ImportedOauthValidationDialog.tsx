import { useMemo, useState } from 'react'
import { AppIcon } from './AppIcon'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Button } from './ui/button'
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from './ui/dialog'
import { cn } from '../lib/utils'
import type {
  ImportedOauthImportResponse,
  ImportedOauthValidationRow,
} from '../lib/api'
import { useTranslation } from '../i18n'

type ValidationFilterKey = 'pending' | 'ok' | 'exhausted' | 'invalid' | 'error' | 'duplicate'

export type ImportedOauthValidationDialogState = {
  inputFiles: number
  uniqueInInput: number
  duplicateInInput: number
  checking: boolean
  importing: boolean
  rows: ImportedOauthValidationRow[]
  importReport?: ImportedOauthImportResponse | null
  importError?: string | null
}

type ImportedOauthValidationDialogProps = {
  open: boolean
  state: ImportedOauthValidationDialogState | null
  onClose: () => void
  onRetryFailed: () => void
  onRetryOne: (sourceId: string) => void
  onImportValid: () => void
}

type ValidationCounts = {
  pending: number
  duplicate: number
  ok: number
  exhausted: number
  invalid: number
  error: number
  checked: number
}

function computeValidationCounts(state: ImportedOauthValidationDialogState | null): ValidationCounts {
  const counts: ValidationCounts = {
    pending: 0,
    duplicate: 0,
    ok: 0,
    exhausted: 0,
    invalid: 0,
    error: 0,
    checked: 0,
  }
  for (const row of state?.rows ?? []) {
    switch (row.status) {
      case 'pending':
        counts.pending += 1
        break
      case 'duplicate_in_input':
        counts.duplicate += 1
        break
      case 'ok':
        counts.ok += 1
        break
      case 'ok_exhausted':
        counts.exhausted += 1
        break
      case 'invalid':
        counts.invalid += 1
        break
      case 'error':
        counts.error += 1
        break
      default:
        counts.error += 1
        break
    }
  }
  counts.checked = counts.duplicate + counts.ok + counts.exhausted + counts.invalid + counts.error
  return counts
}

function filterKeyForStatus(status: ImportedOauthValidationRow['status']): ValidationFilterKey {
  switch (status) {
    case 'pending':
      return 'pending'
    case 'duplicate_in_input':
      return 'duplicate'
    case 'ok':
      return 'ok'
    case 'ok_exhausted':
      return 'exhausted'
    case 'invalid':
      return 'invalid'
    case 'error':
    default:
      return 'error'
  }
}

function rowBadgeVariant(status: ImportedOauthValidationRow['status']) {
  switch (status) {
    case 'ok':
      return 'success' as const
    case 'ok_exhausted':
      return 'warning' as const
    case 'pending':
      return 'info' as const
    case 'duplicate_in_input':
      return 'secondary' as const
    case 'invalid':
    case 'error':
    default:
      return 'error' as const
  }
}

function rowStatusTone(status: ImportedOauthValidationRow['status']) {
  switch (status) {
    case 'ok':
      return 'border-success/30 bg-success/8'
    case 'ok_exhausted':
      return 'border-warning/30 bg-warning/8'
    case 'pending':
      return 'border-info/30 bg-info/8'
    case 'duplicate_in_input':
      return 'border-base-300 bg-base-200/50'
    case 'invalid':
    case 'error':
    default:
      return 'border-error/30 bg-error/8'
  }
}

function formatStatusLabel(
  t: (key: string, values?: Record<string, string | number>) => string,
  status: ImportedOauthValidationRow['status'],
) {
  switch (status) {
    case 'pending':
      return t('accountPool.upstreamAccounts.import.validation.status.pending')
    case 'duplicate_in_input':
      return t('accountPool.upstreamAccounts.import.validation.status.duplicate')
    case 'ok':
      return t('accountPool.upstreamAccounts.import.validation.status.ok')
    case 'ok_exhausted':
      return t('accountPool.upstreamAccounts.import.validation.status.exhausted')
    case 'invalid':
      return t('accountPool.upstreamAccounts.import.validation.status.invalid')
    case 'error':
    default:
      return t('accountPool.upstreamAccounts.import.validation.status.error')
  }
}

function formatFilterLabel(
  t: (key: string, values?: Record<string, string | number>) => string,
  filter: ValidationFilterKey,
) {
  return formatStatusLabel(
    t,
    filter === 'duplicate'
      ? 'duplicate_in_input'
      : filter === 'exhausted'
        ? 'ok_exhausted'
        : filter,
  )
}

export function ImportedOauthValidationDialog({
  open,
  state,
  onClose,
  onRetryFailed,
  onRetryOne,
  onImportValid,
}: ImportedOauthValidationDialogProps) {
  const { t } = useTranslation()
  const [activeFilter, setActiveFilter] = useState<ValidationFilterKey | null>(null)
  const counts = useMemo(() => computeValidationCounts(state), [state])
  const validRows = useMemo(
    () => (state?.rows ?? []).filter((row) => row.status === 'ok' || row.status === 'ok_exhausted'),
    [state],
  )
  const filteredRows = useMemo(() => {
    const rows = state?.rows ?? []
    if (!activeFilter) return rows
    return rows.filter((row) => filterKeyForStatus(row.status) === activeFilter)
  }, [activeFilter, state])
  const isBusy = state?.checking === true || state?.importing === true
  const canRetryFailed = !isBusy && (counts.invalid > 0 || counts.error > 0)
  const canImportValid = !isBusy && validRows.length > 0
  const totalSegments = Math.max(1, state?.uniqueInInput ?? 0)

  const segments: Array<{ key: ValidationFilterKey; count: number; tone: string }> = [
    { key: 'pending', count: counts.pending, tone: 'bg-info' },
    { key: 'ok', count: counts.ok, tone: 'bg-success' },
    { key: 'exhausted', count: counts.exhausted, tone: 'bg-warning' },
    { key: 'invalid', count: counts.invalid, tone: 'bg-error' },
    { key: 'error', count: counts.error, tone: 'bg-error/70' },
  ]

  return (
    <Dialog open={open} onOpenChange={(nextOpen: boolean) => (!nextOpen ? onClose() : undefined)}>
      <DialogContent className="max-h-[88vh] overflow-hidden p-0 sm:max-w-[72rem]">
        <div className="flex max-h-[88vh] flex-col">
          <DialogHeader className="border-b border-base-300 px-6 pb-4 pt-5">
            <div className="flex items-start justify-between gap-4">
              <div className="min-w-0">
                <DialogTitle>{t('accountPool.upstreamAccounts.import.validation.title')}</DialogTitle>
                <DialogDescription className="mt-1">
                  {t('accountPool.upstreamAccounts.import.validation.description', {
                    checked: counts.checked,
                    total: state?.uniqueInInput ?? 0,
                    files: state?.inputFiles ?? 0,
                  })}
                </DialogDescription>
              </div>
              <DialogCloseIcon />
            </div>
            <div className="mt-4 overflow-hidden rounded-full bg-base-200">
              <div className="flex h-2 w-full">
                {segments.map((segment) =>
                  segment.count > 0 ? (
                    <span
                      key={segment.key}
                      className={cn('h-full', segment.tone)}
                      style={{ width: `${(segment.count / totalSegments) * 100}%` }}
                    />
                  ) : null,
                )}
              </div>
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              {[
                ['pending', counts.pending],
                ['ok', counts.ok],
                ['exhausted', counts.exhausted],
                ['invalid', counts.invalid],
                ['error', counts.error],
                ['duplicate', counts.duplicate],
              ].map(([key, count]) => (
                <Button
                  key={key}
                  type="button"
                  variant={activeFilter === key ? 'secondary' : 'ghost'}
                  size="sm"
                  className="rounded-full"
                  onClick={() => setActiveFilter((current) => (current === key ? null : (key as ValidationFilterKey)))}
                >
                  {formatFilterLabel(t, key as ValidationFilterKey)}
                  <span className="ml-2 font-mono text-xs">{count}</span>
                </Button>
              ))}
            </div>
          </DialogHeader>

          <div className="flex-1 overflow-y-auto px-6 py-5">
            {state?.importError ? (
              <Alert variant="error" className="mb-4">
                <AppIcon name="alert-outline" className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <div className="text-sm">{state.importError}</div>
              </Alert>
            ) : null}

            {state?.importReport ? (
              <div className="mb-4 rounded-[1.2rem] border border-base-300 bg-base-100 p-4">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="text-sm font-semibold text-base-content">
                    {t('accountPool.upstreamAccounts.import.validation.reportTitle')}
                  </h3>
                  <Badge variant="success">{t('accountPool.upstreamAccounts.import.validation.reportReady')}</Badge>
                </div>
                <div className="mt-3 grid gap-3 text-sm sm:grid-cols-4">
                  <div>
                    <p className="text-base-content/60">{t('accountPool.upstreamAccounts.import.validation.report.created')}</p>
                    <p className="font-mono text-base-content">{state.importReport.summary.created}</p>
                  </div>
                  <div>
                    <p className="text-base-content/60">{t('accountPool.upstreamAccounts.import.validation.report.updated')}</p>
                    <p className="font-mono text-base-content">{state.importReport.summary.updatedExisting}</p>
                  </div>
                  <div>
                    <p className="text-base-content/60">{t('accountPool.upstreamAccounts.import.validation.report.failed')}</p>
                    <p className="font-mono text-base-content">{state.importReport.summary.failed}</p>
                  </div>
                  <div>
                    <p className="text-base-content/60">{t('accountPool.upstreamAccounts.import.validation.report.selected')}</p>
                    <p className="font-mono text-base-content">{state.importReport.summary.selectedFiles}</p>
                  </div>
                </div>
              </div>
            ) : null}

            {filteredRows.length === 0 ? (
              <div className="rounded-[1.2rem] border border-dashed border-base-300 bg-base-100 px-4 py-8 text-center text-sm text-base-content/65">
                {state?.checking
                  ? t('accountPool.upstreamAccounts.import.validation.checking')
                  : t('accountPool.upstreamAccounts.import.validation.empty')}
              </div>
            ) : (
              <div className="grid gap-3">
                {filteredRows.map((row) => {
                  const canRetryOne = !isBusy && (row.status === 'invalid' || row.status === 'error')
                  return (
                    <div
                      key={row.sourceId}
                      className={cn(
                        'rounded-[1.2rem] border px-4 py-4 shadow-sm',
                        rowStatusTone(row.status),
                      )}
                    >
                      <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-2">
                            <p className="truncate text-sm font-semibold text-base-content">{row.fileName}</p>
                            <Badge variant={rowBadgeVariant(row.status)}>
                              {formatStatusLabel(t, row.status)}
                            </Badge>
                            {row.matchedAccount ? (
                              <Badge variant="secondary">
                                {t('accountPool.upstreamAccounts.import.validation.matchedAccount', {
                                  name: row.matchedAccount.displayName,
                                })}
                              </Badge>
                            ) : null}
                          </div>
                          <div className="mt-2 grid gap-2 text-sm text-base-content/78 sm:grid-cols-2">
                            <p className="truncate">
                              <span className="text-base-content/55">{t('accountPool.upstreamAccounts.fields.email')}:</span>{' '}
                              {row.email || '—'}
                            </p>
                            <p className="truncate">
                              <span className="text-base-content/55">{t('accountPool.upstreamAccounts.fields.accountId')}:</span>{' '}
                              {row.chatgptAccountId || '—'}
                            </p>
                            <p className="truncate">
                              <span className="text-base-content/55">{t('accountPool.upstreamAccounts.fields.displayName')}:</span>{' '}
                              {row.displayName || '—'}
                            </p>
                            <p className="truncate">
                              <span className="text-base-content/55">{t('accountPool.upstreamAccounts.fields.tokenExpiresAt')}:</span>{' '}
                              {row.tokenExpiresAt || '—'}
                            </p>
                          </div>
                          {row.detail ? (
                            <p className="mt-3 text-sm leading-6 text-base-content/72">{row.detail}</p>
                          ) : null}
                        </div>
                        <div className="flex shrink-0 items-center gap-2">
                          {row.attempts > 0 ? (
                            <Badge variant="secondary" className="font-mono">
                              {t('accountPool.upstreamAccounts.import.validation.attempts', {
                                count: row.attempts,
                              })}
                            </Badge>
                          ) : null}
                          {canRetryOne ? (
                            <Button type="button" variant="outline" size="sm" onClick={() => onRetryOne(row.sourceId)}>
                              <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                              {t('accountPool.upstreamAccounts.import.validation.retryOne')}
                            </Button>
                          ) : null}
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </div>

          <DialogFooter className="border-t border-base-300 px-6 py-4">
            <div className="flex w-full flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <p className="text-sm text-base-content/65">
                {t('accountPool.upstreamAccounts.import.validation.footerHint', {
                  valid: validRows.length,
                  duplicates: state?.duplicateInInput ?? 0,
                })}
              </p>
              <div className="flex flex-wrap justify-end gap-2">
                <Button type="button" variant="ghost" onClick={onClose}>
                  {t('accountPool.upstreamAccounts.actions.cancel')}
                </Button>
                <Button type="button" variant="outline" onClick={onRetryFailed} disabled={!canRetryFailed}>
                  <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                  {t('accountPool.upstreamAccounts.import.validation.retryFailed')}
                </Button>
                <Button type="button" onClick={onImportValid} disabled={!canImportValid}>
                  {state?.importing ? <SpinnerInline /> : <AppIcon name="content-save-plus-outline" className="mr-2 h-4 w-4" aria-hidden />}
                  {t('accountPool.upstreamAccounts.import.validation.importValid', { count: validRows.length })}
                </Button>
              </div>
            </div>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  )
}

function SpinnerInline() {
  return <span className="mr-2 h-4 w-4 animate-spin rounded-full border-2 border-primary-content/35 border-t-primary-content" />
}
