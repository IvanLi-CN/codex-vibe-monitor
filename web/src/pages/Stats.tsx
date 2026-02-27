import { useEffect, useMemo, useState } from 'react'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { SuccessFailureChart } from '../components/SuccessFailureChart'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'
import { useErrorDistribution } from '../hooks/useErrorDistribution'
import { useFailureSummary } from '../hooks/useFailureSummary'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { ErrorReasonPieChart } from '../components/ErrorReasonPieChart'
import { Alert } from '../components/ui/alert'
import type { FailureScope } from '../lib/api'

const RANGE_OPTIONS = [
  { value: '1h', labelKey: 'stats.range.lastHour' },
  { value: 'today', labelKey: 'stats.range.today' },
  { value: '1d', labelKey: 'stats.range.lastDay' },
  { value: 'thisWeek', labelKey: 'stats.range.thisWeek' },
  { value: '7d', labelKey: 'stats.range.lastWeek' },
  { value: 'thisMonth', labelKey: 'stats.range.thisMonth' },
  { value: '1mo', labelKey: 'stats.range.lastMonth' },
] as const satisfies readonly { value: string; labelKey: TranslationKey }[]

const BUCKET_OPTION_KEYS: Record<string, { value: string; labelKey: TranslationKey }[]> = {
  '1h': [
    { value: '1m', labelKey: 'stats.bucket.eachMinute' },
    { value: '5m', labelKey: 'stats.bucket.each5Minutes' },
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
  ],
  '1d': [
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
    { value: '30m', labelKey: 'stats.bucket.each30Minutes' },
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
  ],
  today: [
    { value: '15m', labelKey: 'stats.bucket.each15Minutes' },
    { value: '30m', labelKey: 'stats.bucket.each30Minutes' },
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
  ],
  '7d': [
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
  ],
  thisWeek: [
    { value: '1h', labelKey: 'stats.bucket.eachHour' },
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
  '1mo': [
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
  thisMonth: [
    { value: '6h', labelKey: 'stats.bucket.each6Hours' },
    { value: '12h', labelKey: 'stats.bucket.each12Hours' },
    { value: '1d', labelKey: 'stats.bucket.eachDay' },
  ],
}

export default function StatsPage() {
  const { t } = useTranslation()
  const [range, setRange] = useState<typeof RANGE_OPTIONS[number]['value']>('today')
  const [errorScope, setErrorScope] = useState<FailureScope>('service')
  const rawBucketOptions = useMemo(() => BUCKET_OPTION_KEYS[range] ?? BUCKET_OPTION_KEYS['1d'], [range])
  const [bucket, setBucket] = useState<string>(rawBucketOptions[0]?.value ?? '1h')

  // Guarantee we never request an incompatible bucket for the selected range.
  // When range changes, the previous bucket (e.g., 15m) may be invalid for 1mo.
  // Compute an effective bucket that always belongs to the current range options.
  const effectiveBucket = useMemo(() => {
    if (rawBucketOptions.some((option) => option.value === bucket)) return bucket
    return rawBucketOptions[0]?.value ?? '1h'
  }, [bucket, rawBucketOptions])

  // Keep internal bucket state in sync after range changes so the select displays correctly
  useEffect(() => {
    if (bucket !== effectiveBucket) setBucket(effectiveBucket)
  }, [bucket, effectiveBucket])

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const bucketOptions = useMemo(
    () => rawBucketOptions.map((option) => ({ ...option, label: t(option.labelKey) })),
    [rawBucketOptions, t],
  )

  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useSummary(range)

  const {
    data: timeseries,
    isLoading: timeseriesLoading,
    error: timeseriesError,
  } = useTimeseries(range, {
    bucket: effectiveBucket,
    preferServerAggregation: true,
  })

  const { data: errors, isLoading: errorsLoading, error: errorsError } = useErrorDistribution(range, 8, errorScope)
  const {
    data: failureSummary,
    isLoading: failureSummaryLoading,
    error: failureSummaryError,
  } = useFailureSummary(range)

  const scopeOptions = useMemo(
    () =>
      [
        { value: 'service', label: t('stats.errors.scope.service') },
        { value: 'client', label: t('stats.errors.scope.client') },
        { value: 'abort', label: t('stats.errors.scope.abort') },
        { value: 'all', label: t('stats.errors.scope.all') },
      ] as const,
    [t],
  )

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('stats.title')}</h2>
              <p className="section-description">{t('stats.subtitle')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <select
                className="field-select field-select-sm min-w-[8.5rem]"
                value={range}
                onChange={(event) => setRange(event.target.value as typeof range)}
              >
                {rangeOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              <select
                className="field-select field-select-sm min-w-[7rem]"
                value={effectiveBucket}
                onChange={(event) => setBucket(event.target.value)}
              >
                {bucketOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>


      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t('stats.trendTitle')}</h3>
          </div>
          {timeseriesError ? (
            <Alert variant="error">{timeseriesError}</Alert>
          ) : (
            <TimeseriesChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="section-heading">
            <h3 className="section-title">{t('stats.successFailureTitle')}</h3>
          </div>
          {timeseriesError ? (
            <Alert variant="error">{timeseriesError}</Alert>
          ) : (
            <SuccessFailureChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h3 className="section-title">{t('stats.errors.title')}</h3>
              {failureSummaryError ? (
                <p className="section-description text-error">{failureSummaryError}</p>
              ) : (
                <p className="section-description">
                  {t('stats.errors.actionableRate', {
                    rate: `${((failureSummary?.actionableFailureRate ?? 0) * 100).toFixed(1)}%`,
                  })}
                </p>
              )}
            </div>
            <label className="field w-full max-w-[14rem]">
              <span className="field-label text-sm">{t('stats.errors.scope.label')}</span>
              <select
                className="field-select field-select-sm"
                value={errorScope}
                onChange={(event) => setErrorScope(event.target.value as FailureScope)}
              >
                {scopeOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
          <div className="metric-grid w-full grid-cols-1 sm:grid-cols-4">
            <div className="metric-cell">
              <div className="metric-label">{t('stats.errors.summary.service')}</div>
              <div className="metric-value text-error text-2xl">
                {failureSummaryLoading ? '—' : failureSummary?.serviceFailureCount ?? 0}
              </div>
            </div>
            <div className="metric-cell">
              <div className="metric-label">{t('stats.errors.summary.client')}</div>
              <div className="metric-value text-warning text-2xl">
                {failureSummaryLoading ? '—' : failureSummary?.clientFailureCount ?? 0}
              </div>
            </div>
            <div className="metric-cell">
              <div className="metric-label">{t('stats.errors.summary.abort')}</div>
              <div className="metric-value text-info text-2xl">
                {failureSummaryLoading ? '—' : failureSummary?.clientAbortCount ?? 0}
              </div>
            </div>
            <div className="metric-cell">
              <div className="metric-label">{t('stats.errors.summary.actionable')}</div>
              <div className="metric-value text-secondary text-2xl">
                {failureSummaryLoading ? '—' : failureSummary?.actionableFailureCount ?? 0}
              </div>
            </div>
          </div>
          {errorsError ? (
            <Alert variant="error">{errorsError}</Alert>
          ) : (
            <ErrorReasonPieChart items={errors?.items ?? []} isLoading={errorsLoading} />
          )}
        </div>
      </section>
    </div>
  )
}
