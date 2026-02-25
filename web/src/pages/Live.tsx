import { useCallback, useMemo, useState } from 'react'
import { InvocationChart } from '../components/InvocationChart'
import { InvocationTable } from '../components/InvocationTable'
import { StatsCards } from '../components/StatsCards'
import { useInvocationStream } from '../hooks/useInvocations'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { cn } from '../lib/utils'

const LIMIT_OPTIONS = [20, 50, 100]
const SUMMARY_WINDOWS: { value: string; labelKey: TranslationKey }[] = [
  { value: 'current', labelKey: 'live.summary.current' },
  { value: '30m', labelKey: 'live.summary.30m' },
  { value: '1h', labelKey: 'live.summary.1h' },
  { value: '1d', labelKey: 'live.summary.1d' },
]

export default function LivePage() {
  const { t } = useTranslation()
  const [limit, setLimit] = useState(50)
  const [summaryWindow, setSummaryWindow] = useState('current')

  const summaryWindows = useMemo(
    () => SUMMARY_WINDOWS.map((option) => ({ value: option.value, label: t(option.labelKey) })),
    [t],
  )

  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
    refresh: refreshSummary,
  } = useSummary(summaryWindow, summaryWindow === 'current' ? { limit } : undefined)

  const handleNewRecords = useCallback(() => {
    void refreshSummary()
  }, [refreshSummary])

  const {
    records,
    isLoading,
    error,
  } = useInvocationStream(limit, undefined, handleNewRecords, { enableStream: true })

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('live.summary.title')}</h2>
            </div>
            <div className="segment-group" role="tablist" aria-label={t('live.summary.title')}>
              {summaryWindows.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="tab"
                  aria-selected={summaryWindow === option.value}
                  aria-pressed={summaryWindow === option.value}
                  onClick={() => setSummaryWindow(option.value)}
                  className={cn('segment-button px-3', summaryWindow === option.value && 'font-semibold')}
                  data-active={summaryWindow === option.value}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-6">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('live.chart.title')}</h2>
            </div>
            <label className="field w-36">
              <span className="field-label">{t('live.window.label')}</span>
              <select
                className="field-select field-select-sm"
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
              >
                {LIMIT_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {t('live.option.records', { count: value })}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <InvocationChart records={records} isLoading={isLoading} />
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h2 className="section-title">{t('live.latest.title')}</h2>
          </div>
          <InvocationTable records={records} isLoading={isLoading} error={error} />
        </div>
      </section>
    </div>
  )
}
