import { useEffect, useMemo, useState } from 'react'
import { Alert } from '../../components/ui/alert'
import { Button } from '../../components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { Input } from '../../components/ui/input'
import { SelectField } from '../../components/ui/select-field'
import { fetchSystemTaskRuns, type SystemTaskRun } from '../../lib/api'
import { useTranslation } from '../../i18n'

const TASK_PAGE_SIZE_OPTIONS = [10, 20, 50, 100].map((value) => ({
  value: String(value),
  label: String(value),
}))

function toIsoStringOrUndefined(value: string, upperBound = false): string | undefined {
  const normalized = value.trim()
  if (!normalized) return undefined
  const parsed = new Date(normalized)
  if (Number.isNaN(parsed.getTime())) return undefined
  if (upperBound && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(normalized)) {
    parsed.setMinutes(parsed.getMinutes() + 1)
  }
  return parsed.toISOString()
}

function statusTone(status: string): string {
  switch (status) {
    case 'success':
      return 'text-success'
    case 'failed':
      return 'text-error'
    case 'skipped':
      return 'text-warning'
    default:
      return 'text-info'
  }
}

export default function SystemTasksPage() {
  const { t } = useTranslation()
  const [items, setItems] = useState<SystemTaskRun[]>([])
  const [total, setTotal] = useState(0)
  const [error, setError] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [taskKind, setTaskKind] = useState('')
  const [status, setStatus] = useState('')
  const [startedAtFrom, setStartedAtFrom] = useState('')
  const [startedAtTo, setStartedAtTo] = useState('')
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(20)

  const startedAtFromIso = useMemo(() => toIsoStringOrUndefined(startedAtFrom), [startedAtFrom])
  const startedAtToIso = useMemo(() => toIsoStringOrUndefined(startedAtTo, true), [startedAtTo])

  useEffect(() => {
    let active = true
    setIsLoading(true)
    fetchSystemTaskRuns({
      taskKind: taskKind || undefined,
      status: status || undefined,
      startedAtFrom: startedAtFromIso,
      startedAtTo: startedAtToIso,
      page,
      pageSize,
    })
      .then((response) => {
        if (!active) return
        setItems(response.items)
        setTotal(response.total)
        setPage(response.page)
        setPageSize(response.pageSize)
        setError(null)
      })
      .catch((err) => {
        if (!active) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (!active) return
        setIsLoading(false)
      })

    return () => {
      active = false
    }
  }, [page, pageSize, startedAtFromIso, startedAtToIso, status, taskKind])

  const filteredCount = useMemo(() => total.toLocaleString(), [total])
  const pageCount = useMemo(() => Math.max(1, Math.ceil(total / pageSize)), [pageSize, total])

  return (
    <section className="surface-panel overflow-hidden">
      <div className="surface-panel-body gap-5">
        <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
          <div className="section-heading">
            <h2 className="section-title text-2xl">{t('system.tasks.title')}</h2>
            <p className="section-description max-w-3xl">{t('system.tasks.description')}</p>
          </div>
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
            <Input
              value={taskKind}
              onChange={(event) => {
                setTaskKind(event.target.value)
                setPage(1)
              }}
              placeholder={t('system.tasks.filters.taskKindPlaceholder')}
            />
            <SelectField
              value={status}
              onValueChange={(value) => {
                setStatus(value)
                setPage(1)
              }}
              options={[
                { value: '', label: t('system.tasks.filters.allStatuses') },
                { value: 'success', label: 'success' },
                { value: 'failed', label: 'failed' },
                { value: 'skipped', label: 'skipped' },
                { value: 'running', label: 'running' },
              ]}
            />
            <label className="space-y-1">
              <span className="text-xs font-medium text-base-content/65">
                {t('system.tasks.filters.startedAtFrom')}
              </span>
              <Input
                type="datetime-local"
                value={startedAtFrom}
                onChange={(event) => {
                  setStartedAtFrom(event.target.value)
                  setPage(1)
                }}
              />
            </label>
            <label className="space-y-1">
              <span className="text-xs font-medium text-base-content/65">
                {t('system.tasks.filters.startedAtTo')}
              </span>
              <Input
                type="datetime-local"
                value={startedAtTo}
                onChange={(event) => {
                  setStartedAtTo(event.target.value)
                  setPage(1)
                }}
              />
            </label>
            <div className="flex items-center rounded-xl border border-base-300/75 bg-base-100/72 px-3 text-sm text-base-content/70">
              {t('system.tasks.filters.count', { count: filteredCount })}
            </div>
          </div>
        </div>

        {error ? <Alert variant="error">{t('system.tasks.loadError', { error })}</Alert> : null}
        {isLoading ? <Alert variant="info">{t('system.tasks.loading')}</Alert> : null}

        <div className="grid gap-4" data-testid="system-tasks-list">
          {items.map((item) => (
            <Card key={item.id} className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
              <CardHeader className="gap-2 border-b border-base-300/70 pb-4">
                <div className="flex flex-col gap-2 md:flex-row md:items-start md:justify-between">
                  <div>
                    <CardTitle className="text-base font-semibold">{item.taskKind}</CardTitle>
                    <CardDescription>{t('system.tasks.meta', { trigger: item.triggerKind, startedAt: item.startedAt })}</CardDescription>
                  </div>
                  <div className={`text-sm font-semibold uppercase tracking-[0.14em] ${statusTone(item.status)}`}>
                    {item.status}
                  </div>
                </div>
              </CardHeader>
              <CardContent className="space-y-2 pt-4 text-sm">
                {item.summary ? <div>{item.summary}</div> : null}
                {item.detail ? <div className="text-base-content/68">{item.detail}</div> : null}
                <div className="text-xs text-base-content/55">
                  {t('system.tasks.duration', {
                    duration: item.durationMs == null ? '—' : `${item.durationMs} ms`,
                    finishedAt: item.finishedAt ?? '—',
                  })}
                </div>
              </CardContent>
            </Card>
          ))}
          {!isLoading && items.length === 0 ? (
            <Alert variant="info">{t('system.tasks.empty')}</Alert>
          ) : null}
        </div>

        <div
          className="flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-end sm:justify-between"
          data-testid="system-tasks-pagination"
        >
          <div className="text-sm text-base-content/70">
            {t('system.tasks.pagination.summary', {
              page,
              pageCount,
              total: total.toLocaleString(),
            })}
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <div className="flex items-center gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2">
              <span className="text-sm font-medium text-base-content/65">{t('system.tasks.pagination.pageSize')}</span>
              <SelectField
                className="min-w-[7rem]"
                value={String(pageSize)}
                options={TASK_PAGE_SIZE_OPTIONS}
                size="sm"
                triggerClassName="h-11 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm lg:h-10"
                aria-label={t('system.tasks.pagination.pageSize')}
                onValueChange={(value) => {
                  setPageSize(Number(value))
                  setPage(1)
                }}
              />
            </div>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-11 rounded-xl px-4 lg:h-10"
                onClick={() => setPage((current) => Math.max(1, current - 1))}
                disabled={isLoading || page <= 1}
              >
                {t('system.tasks.pagination.previous')}
              </Button>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-11 rounded-xl px-4 lg:h-10"
                onClick={() => setPage((current) => Math.min(pageCount, current + 1))}
                disabled={isLoading || page >= pageCount}
              >
                {t('system.tasks.pagination.next')}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </section>
  )
}
