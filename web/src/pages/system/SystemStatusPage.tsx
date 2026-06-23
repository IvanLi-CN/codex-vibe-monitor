import { useEffect, useMemo, useState } from 'react'
import { Alert } from '../../components/ui/alert'
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

type MetricCellProps = {
  title: string
  value: string
  hint?: string
  tone?: 'default' | 'primary' | 'secondary'
  badge?: string
}

function MetricCell({ title, value, hint, tone = 'default', badge }: MetricCellProps) {
  const toneClass =
    tone === 'primary'
      ? 'text-primary'
      : tone === 'secondary'
        ? 'text-secondary'
        : 'text-base-content'
  return (
    <div className="metric-cell h-full">
      <div className="flex flex-wrap items-center gap-2">
        <div className="metric-label normal-case tracking-normal">{title}</div>
        {badge ? (
          <span className="rounded-full border border-base-300/75 bg-base-100/80 px-2 py-0.5 text-[11px] font-semibold text-base-content/72">
            {badge}
          </span>
        ) : null}
      </div>
      <div className={`metric-value mt-2 text-2xl tabular-nums sm:text-3xl ${toneClass}`}>{value}</div>
      {hint ? <div className="mt-2 text-xs leading-relaxed text-base-content/65">{hint}</div> : null}
    </div>
  )
}

type BreakdownRowProps = {
  label: string
  value: string
  hint?: string
}

function BreakdownRow({ label, value, hint }: BreakdownRowProps) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-lg border border-base-300/70 bg-base-100/50 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-semibold text-base-content">{label}</div>
        {hint ? <div className="mt-1 text-xs leading-relaxed text-base-content/65">{hint}</div> : null}
      </div>
      <div className="shrink-0 text-right text-lg font-semibold tabular-nums text-base-content sm:text-xl">{value}</div>
    </div>
  )
}

type PairedMetricProps = {
  title: string
  testId?: string
  badge?: string
  summary?: string
  bytesLabel: string
  bytesValue: string
  countLabel: string
  countValue: string
  bytesHint?: string
  countHint?: string
  tone?: 'default' | 'secondary'
}

function PairedMetric({
  title,
  testId,
  badge,
  summary,
  bytesLabel,
  bytesValue,
  countLabel,
  countValue,
  bytesHint,
  countHint,
  tone = 'default',
}: PairedMetricProps) {
  return (
    <div
      className="rounded-xl border border-base-300/75 bg-base-100/60 px-4 py-4"
      data-testid={testId}
    >
      <div className="flex flex-wrap items-center gap-2">
        <div className="text-sm font-semibold text-base-content">{title}</div>
        {badge ? (
          <span className="rounded-full border border-base-300/75 bg-base-100/80 px-2 py-0.5 text-[11px] font-semibold text-base-content/72">
            {badge}
          </span>
        ) : null}
      </div>
      {summary ? <p className="mt-2 max-w-[44ch] text-xs leading-relaxed text-base-content/65">{summary}</p> : null}
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <MetricCell title={bytesLabel} value={bytesValue} hint={bytesHint} tone={tone} />
        <MetricCell title={countLabel} value={countValue} hint={countHint} />
      </div>
    </div>
  )
}

type OverviewPanelProps = {
  status: SystemStatusResponse
  t: ReturnType<typeof useTranslation>['t']
}

function OverviewPanel({ status, t }: OverviewPanelProps) {
  const projectDiskBytes =
    status.archivedBodies.bytes + status.rawBodies.bytes + status.databaseBytes + status.otherFilesBytes

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-status-overview">
      <div className="surface-panel-body gap-5">
        <div className="section-heading">
          <h3 className="section-title">{t('system.status.sections.diskOverviewTitle')}</h3>
          <p className="section-description max-w-[65ch]">{t('system.status.sections.diskOverviewDescription')}</p>
        </div>

        <div className="rounded-xl border border-primary/20 bg-primary/8 px-5 py-5">
          <div className="text-sm font-semibold text-primary">{t('system.status.summary.projectDiskLabel')}</div>
          <div className="mt-2 text-4xl font-semibold tracking-tight tabular-nums text-base-content sm:text-5xl">
            {formatBytes(projectDiskBytes)}
          </div>
          <p className="mt-3 max-w-[60ch] text-sm leading-relaxed text-base-content/72">
            {t('system.status.summary.projectDiskHint')}
          </p>
          <p
            className="mt-3 max-w-[65ch] text-xs font-medium leading-relaxed text-base-content/75"
            data-testid="system-status-project-disk-formula"
          >
            {t('system.status.summary.projectDiskFormula')}
          </p>
        </div>

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <BreakdownRow
            label={t('system.status.breakdown.rawPayloadBytes')}
            value={formatBytes(status.rawBodies.bytes)}
            hint={t('system.status.breakdown.rawPayloadBytesHint')}
          />
          <BreakdownRow
            label={t('system.status.breakdown.archiveBytes')}
            value={formatBytes(status.archivedBodies.bytes)}
            hint={t('system.status.breakdown.archiveBytesHint')}
          />
          <BreakdownRow
            label={t('system.status.breakdown.databaseBytes')}
            value={formatBytes(status.databaseBytes)}
            hint={t('system.status.breakdown.databaseBytesHint')}
          />
          <BreakdownRow
            label={t('system.status.breakdown.otherFilesBytes')}
            value={formatBytes(status.otherFilesBytes)}
            hint={t('system.status.breakdown.otherFilesBytesHint')}
          />
        </div>

        <div className="rounded-xl border border-base-300/75 bg-base-100/58 px-5 py-5">
          <div className="section-heading">
            <h3 className="section-title">{t('system.status.sections.rawPayloadFocusTitle')}</h3>
            <p className="section-description max-w-[70ch]">{t('system.status.sections.rawPayloadFocusDescription')}</p>
          </div>
          <div className="mt-5 grid gap-3 xl:grid-cols-[minmax(0,18rem)_minmax(0,1fr)] xl:items-start">
            <MetricCell
              title={t('system.status.cards.rawBodiesBytes')}
              value={formatBytes(status.rawBodies.bytes)}
              hint={t('system.status.cards.rawBodiesBytesHint')}
              tone="primary"
              badge={t('system.status.metric.unionBadge')}
            />
            <div className="grid gap-3 xl:grid-cols-2">
              <PairedMetric
                title={t('system.status.cards.requestRawBodiesBytes')}
                testId="system-status-request-raw-breakdown"
                badge={t('system.status.metric.splitBadge')}
                summary={t('system.status.cards.requestRawBodiesSplitHint')}
                bytesLabel={t('system.status.metric.bytesLabel')}
                bytesValue={formatBytes(status.requestRawBodies.bytes)}
                countLabel={t('system.status.metric.countLabel')}
                countValue={status.requestRawBodies.count.toLocaleString()}
                tone="secondary"
              />
              <PairedMetric
                title={t('system.status.cards.responseRawBodiesBytes')}
                testId="system-status-response-raw-breakdown"
                badge={t('system.status.metric.splitBadge')}
                summary={t('system.status.cards.responseRawBodiesSplitHint')}
                bytesLabel={t('system.status.metric.bytesLabel')}
                bytesValue={formatBytes(status.responseRawBodies.bytes)}
                countLabel={t('system.status.metric.countLabel')}
                countValue={status.responseRawBodies.count.toLocaleString()}
              />
            </div>
          </div>
          <div className="mt-3">
            <Alert variant="info">{t('system.status.rawPayloadDefinition')}</Alert>
          </div>
        </div>
      </div>
    </section>
  )
}

type MetricSectionProps = {
  title: string
  description: string
  metrics: MetricCellProps[]
  testId: string
}

function MetricSection({ title, description, metrics, testId }: MetricSectionProps) {
  return (
    <section className="surface-panel overflow-hidden" data-testid={testId}>
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h3 className="section-title">{title}</h3>
          <p className="section-description max-w-[65ch]">{description}</p>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          {metrics.map((metric) => (
            <MetricCell key={metric.title} {...metric} />
          ))}
        </div>
      </div>
    </section>
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

  const sections = useMemo(() => {
    if (!status) return null

    return {
      databaseMetrics: [
        {
          title: t('system.status.cards.liveInvocationsCount'),
          value: status.liveInvocationsCount.toLocaleString(),
          hint: t('system.status.cards.liveInvocationsCountHint'),
          tone: 'primary' as const,
        },
        {
          title: t('system.status.cards.successCount'),
          value: status.successCount.toLocaleString(),
          hint: t('system.status.cards.successCountHint'),
        },
        {
          title: t('system.status.cards.nonSuccessCount'),
          value: status.nonSuccessCount.toLocaleString(),
          hint: t('system.status.cards.nonSuccessCountHint'),
        },
        {
          title: t('system.status.cards.completedArchiveBatchesCount'),
          value: status.completedArchiveBatchesCount.toLocaleString(),
          hint: t('system.status.cards.completedArchiveBatchesCountHint'),
        },
      ],
      archiveMetrics: [
        {
          title: t('system.status.cards.archivedBodiesCount'),
          value: status.archivedBodies.count.toLocaleString(),
          hint: t('system.status.cards.archivedBodiesCountHint'),
        },
        {
          title: t('system.status.cards.archivedBodiesBytes'),
          value: formatBytes(status.archivedBodies.bytes),
          hint: t('system.status.cards.archivedBodiesBytesHint'),
          tone: 'secondary' as const,
        },
        {
          title: t('system.status.cards.rawBodiesCount'),
          value: status.rawBodies.count.toLocaleString(),
          hint: t('system.status.cards.rawBodiesCountHint'),
        },
      ],
    }
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
            <div className="space-y-4" data-testid="system-status-layout">
              <OverviewPanel status={status} t={t} />
              <div className="grid gap-4 xl:grid-cols-2" data-testid="system-status-sections">
                <MetricSection
                  testId="system-status-records-section"
                  title={t('system.status.sections.databaseRecordsTitle')}
                  description={t('system.status.sections.databaseRecordsDescription')}
                  metrics={sections?.databaseMetrics ?? []}
                />
                <MetricSection
                  testId="system-status-archive-section"
                  title={t('system.status.sections.archiveLogicalTitle')}
                  description={t('system.status.sections.archiveLogicalDescription')}
                  metrics={sections?.archiveMetrics ?? []}
                />
              </div>
            </div>
          ) : null}
          <Alert variant="info">{t('system.status.definition')}</Alert>
        </div>
      </section>
    </div>
  )
}
