import { useEffect, useMemo, useState } from 'react'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { SuccessFailureChart } from '../components/SuccessFailureChart'
import { ParallelWorkStatsSection } from '../components/ParallelWorkStatsSection'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'
import { useErrorDistribution } from '../hooks/useErrorDistribution'
import { useFailureSummary } from '../hooks/useFailureSummary'
import { useParallelWorkStats } from '../hooks/useParallelWorkStats'
import { useTranslation } from '../i18n'
import { ErrorReasonPieChart } from '../components/ErrorReasonPieChart'
import { Alert } from '../components/ui/alert'
import { SelectField } from '../components/ui/select-field'
import type { FailureScope } from '../lib/api'
import {
  resolveStatsBucketOptions,
  resolveStatsBucketValue,
  RANGE_OPTIONS,
} from './stats-options'

export default function StatsPage() {
  const { t } = useTranslation()
  const [range, setRange] = useState<typeof RANGE_OPTIONS[number]['value']>('today')
  const [errorScope, setErrorScope] = useState<FailureScope>('service')
  const [bucket, setBucket] = useState<string>(() =>
    resolveStatsBucketValue('', resolveStatsBucketOptions('today')),
  )

  const requestedBucketOptions = useMemo(
    () => resolveStatsBucketOptions(range),
    [range],
  )
  const requestedBucket = useMemo(
    () => resolveStatsBucketValue(bucket, requestedBucketOptions),
    [bucket, requestedBucketOptions],
  )

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
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
    bucket: requestedBucket,
    preferServerAggregation: true,
  })

  const rawBucketOptions = useMemo(
    () => resolveStatsBucketOptions(range, timeseries?.availableBuckets),
    [range, timeseries?.availableBuckets],
  )
  const effectiveBucket = useMemo(
    () => {
      const serverBucket = timeseries?.effectiveBucket
      const requestedStillAllowed = rawBucketOptions.some((option) => option.value === requestedBucket)
      // Keep the user's in-flight selection visible until the server explicitly narrows it.
      if (requestedStillAllowed) {
        return requestedBucket
      }
      return resolveStatsBucketValue(serverBucket ?? requestedBucket, rawBucketOptions)
    },
    [rawBucketOptions, requestedBucket, timeseries?.effectiveBucket],
  )
  const bucketOptions = useMemo(
    () => rawBucketOptions.map((option) => ({ ...option, label: t(option.labelKey) })),
    [rawBucketOptions, t],
  )

  // Keep internal bucket state in sync after the backend narrows unsupported options.
  useEffect(() => {
    if (bucket !== effectiveBucket) setBucket(effectiveBucket)
  }, [bucket, effectiveBucket])

  const { data: errors, isLoading: errorsLoading, error: errorsError } = useErrorDistribution(range, 8, errorScope)
  const {
    data: failureSummary,
    isLoading: failureSummaryLoading,
    error: failureSummaryError,
  } = useFailureSummary(range)
  const {
    data: parallelWorkStats,
    isLoading: parallelWorkLoading,
    error: parallelWorkError,
  } = useParallelWorkStats()

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
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('stats.title')}</h2>
              <p className="section-description">{t('stats.subtitle')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <SelectField
                options={rangeOptions}
                value={range}
                onValueChange={(value) => setRange(value as typeof range)}
                triggerClassName="min-w-[8.5rem]"
                data-testid="stats-range-select-trigger"
                aria-label={t('stats.subtitle')}
              />
              <SelectField
                options={bucketOptions}
                value={effectiveBucket}
                onValueChange={setBucket}
                triggerClassName="min-w-[7rem]"
                data-testid="stats-bucket-select-trigger"
                aria-label={t('stats.trendTitle')}
              />
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

      <ParallelWorkStatsSection
        stats={parallelWorkStats}
        isLoading={parallelWorkLoading}
        error={parallelWorkError}
      />

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
            <SelectField
              label={t('stats.errors.scope.label')}
              className="w-full max-w-[14rem]"
              options={scopeOptions}
              value={errorScope}
              onValueChange={(value) => setErrorScope(value as FailureScope)}
              data-testid="stats-error-scope-select-trigger"
              aria-label={t('stats.errors.scope.label')}
            />
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
