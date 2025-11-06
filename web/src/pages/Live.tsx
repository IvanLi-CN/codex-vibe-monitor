import { useCallback, useMemo, useState } from 'react'
import { InvocationChart } from '../components/InvocationChart'
import { InvocationTable } from '../components/InvocationTable'
import { StatsCards } from '../components/StatsCards'
import { useInvocationStream } from '../hooks/useInvocations'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'

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
      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h2 className="card-title">{t('live.summary.title')}</h2>
            <div className="join">
              {summaryWindows.map((option) => (
                <input
                  key={option.value}
                  type="radio"
                  name="summary-window"
                  aria-label={option.label}
                  className="btn join-item"
                  value={option.value}
                  checked={summaryWindow === option.value}
                  onChange={() => setSummaryWindow(option.value)}
                />
              ))}
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-6">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h2 className="card-title">{t('live.chart.title')}</h2>
            <label className="form-control w-36">
              <div className="label py-0">
                <span className="label-text text-xs uppercase tracking-wide">{t('live.window.label')}</span>
              </div>
              <select
                className="select select-bordered select-sm"
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

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <h2 className="card-title">{t('live.latest.title')}</h2>
          <InvocationTable records={records} isLoading={isLoading} error={error} />
        </div>
      </section>
    </div>
  )
}
