import { useEffect, useMemo, useState } from 'react'
import { Alert } from '../../components/ui/alert'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card'
import { fetchSystemStatus, type SystemStatusResponse } from '../../lib/api'
import { useTranslation } from '../../i18n'

const REFRESH_INTERVAL_MS = 60_000

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let current = value
  let index = 0
  while (current >= 1024 && index < units.length - 1) {
    current /= 1024
    index += 1
  }
  return `${current >= 10 || index === 0 ? current.toFixed(0) : current.toFixed(1)} ${units[index]}`
}

type MetricCardProps = {
  title: string
  value: string
  hint: string
}

function MetricCard({ title, value, hint }: MetricCardProps) {
  return (
    <Card className="overflow-hidden border-base-300/75 bg-base-100/92 shadow-sm">
      <CardHeader className="gap-1 pb-3">
        <CardDescription>{title}</CardDescription>
        <CardTitle className="text-3xl font-semibold tabular-nums">{value}</CardTitle>
      </CardHeader>
      <CardContent className="pt-0 text-xs text-base-content/65">{hint}</CardContent>
    </Card>
  )
}

export default function SystemStatusPage() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<SystemStatusResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isRefreshing, setIsRefreshing] = useState(false)

  useEffect(() => {
    let active = true

    const load = async (background: boolean) => {
      if (!background) {
        setIsLoading(true)
      } else {
        setIsRefreshing(true)
      }
      const complete = () => {
        setIsLoading(false)
        setIsRefreshing(false)
      }
      try {
        const next = await fetchSystemStatus()
        if (!active) {
          complete()
          return
        }
        setStatus(next)
        setError(null)
      } catch (err) {
        if (!active) {
          complete()
          return
        }
        setError(err instanceof Error ? err.message : String(err))
      }
      complete()
    }

    void load(false)
    const timer = window.setInterval(() => {
      void load(true)
    }, REFRESH_INTERVAL_MS)

    return () => {
      active = false
      window.clearInterval(timer)
    }
  }, [])

  const cards = useMemo(() => {
    if (!status) return []
    return [
      {
        key: 'success',
        title: t('system.status.cards.successCount'),
        value: status.successCount.toLocaleString(),
        hint: t('system.status.cards.successCountHint'),
      },
      {
        key: 'nonSuccess',
        title: t('system.status.cards.nonSuccessCount'),
        value: status.nonSuccessCount.toLocaleString(),
        hint: t('system.status.cards.nonSuccessCountHint'),
      },
      {
        key: 'archivedCount',
        title: t('system.status.cards.archivedBodiesCount'),
        value: status.archivedBodies.count.toLocaleString(),
        hint: t('system.status.cards.archivedBodiesCountHint'),
      },
      {
        key: 'archivedBytes',
        title: t('system.status.cards.archivedBodiesBytes'),
        value: formatBytes(status.archivedBodies.bytes),
        hint: t('system.status.cards.archivedBodiesBytesHint'),
      },
      {
        key: 'rawCount',
        title: t('system.status.cards.rawBodiesCount'),
        value: status.rawBodies.count.toLocaleString(),
        hint: t('system.status.cards.rawBodiesCountHint'),
      },
      {
        key: 'rawBytes',
        title: t('system.status.cards.rawBodiesBytes'),
        value: formatBytes(status.rawBodies.bytes),
        hint: t('system.status.cards.rawBodiesBytesHint'),
      },
      {
        key: 'requestRawCount',
        title: t('system.status.cards.requestRawBodiesCount'),
        value: status.requestRawBodies.count.toLocaleString(),
        hint: t('system.status.cards.requestRawBodiesCountHint'),
      },
      {
        key: 'requestRawBytes',
        title: t('system.status.cards.requestRawBodiesBytes'),
        value: formatBytes(status.requestRawBodies.bytes),
        hint: t('system.status.cards.requestRawBodiesBytesHint'),
      },
      {
        key: 'responseRawCount',
        title: t('system.status.cards.responseRawBodiesCount'),
        value: status.responseRawBodies.count.toLocaleString(),
        hint: t('system.status.cards.responseRawBodiesCountHint'),
      },
      {
        key: 'responseRawBytes',
        title: t('system.status.cards.responseRawBodiesBytes'),
        value: formatBytes(status.responseRawBodies.bytes),
        hint: t('system.status.cards.responseRawBodiesBytesHint'),
      },
      {
        key: 'database',
        title: t('system.status.cards.databaseBytes'),
        value: formatBytes(status.databaseBytes),
        hint: t('system.status.cards.databaseBytesHint'),
      },
      {
        key: 'otherFiles',
        title: t('system.status.cards.otherFilesBytes'),
        value: formatBytes(status.otherFilesBytes),
        hint: t('system.status.cards.otherFilesBytesHint'),
      },
    ]
  }, [status, t])

  return (
    <div className="space-y-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
            <div className="section-heading">
              <h2 className="section-title text-2xl">{t('system.status.title')}</h2>
              <p className="section-description max-w-3xl">{t('system.status.description')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-2 text-xs text-base-content/65">
              <span>{isRefreshing ? t('system.status.refreshing') : t('system.status.idle')}</span>
              <span>{status ? t('system.status.lastRefreshed', { at: status.refreshedAt }) : t('system.status.lastRefreshedEmpty')}</span>
            </div>
          </div>

          {error && <Alert variant="error">{t('system.status.loadError', { error })}</Alert>}
          {isLoading && !status ? <Alert variant="info">{t('system.status.loading')}</Alert> : null}
          {status ? (
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4" data-testid="system-status-grid">
              {cards.map((card) => (
                <MetricCard key={card.key} title={card.title} value={card.value} hint={card.hint} />
              ))}
            </div>
          ) : null}
          <Alert variant="info">{t('system.status.definition')}</Alert>
        </div>
      </section>
    </div>
  )
}
