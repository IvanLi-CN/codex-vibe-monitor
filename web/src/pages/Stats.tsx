import { useEffect, useMemo, useState } from 'react'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { SuccessFailureChart } from '../components/SuccessFailureChart'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'
import { useErrorDistribution } from '../hooks/useErrorDistribution'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { ErrorReasonPieChart } from '../components/ErrorReasonPieChart'

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

const BUCKET_SECONDS: Record<string, number> = {
  '1m': 60,
  '5m': 300,
  '15m': 900,
  '30m': 1_800,
  '1h': 3_600,
  '6h': 21_600,
  '12h': 43_200,
  '1d': 86_400,
}

export default function StatsPage() {
  const { t } = useTranslation()
  const [range, setRange] = useState<typeof RANGE_OPTIONS[number]['value']>('today')
  const rawBucketOptions = useMemo(() => BUCKET_OPTION_KEYS[range] ?? BUCKET_OPTION_KEYS['1d'], [range])
  const [bucket, setBucket] = useState<string>(rawBucketOptions[0]?.value ?? '1h')
  const [settlementHour, setSettlementHour] = useState(0)

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

  const needsSettlement = BUCKET_SECONDS[effectiveBucket] >= 86_400

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
    settlementHour: needsSettlement ? settlementHour : undefined,
  })

  const { data: errors, isLoading: errorsLoading, error: errorsError } = useErrorDistribution(range, 8)

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="card-heading">
              <h2 className="card-title">{t('stats.title')}</h2>
              <p className="card-description">{t('stats.subtitle')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <select
                className="select select-bordered select-sm min-w-[8.5rem]"
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
                className="select select-bordered select-sm min-w-[7rem]"
                value={effectiveBucket}
                onChange={(event) => setBucket(event.target.value)}
              >
                {bucketOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
              {needsSettlement && (
                <label className="form-control w-28">
                  <div className="label py-0">
                    <span className="label-text text-xs">{t('stats.settlementHour')}</span>
                  </div>
                  <input
                    type="number"
                    min={0}
                    max={23}
                    value={settlementHour}
                    onChange={(event) => setSettlementHour(Number(event.target.value))}
                    className="input input-bordered input-sm"
                  />
                </label>
              )}
            </div>
          </div>
          <StatsCards stats={summary} loading={summaryLoading} error={summaryError} />
        </div>
      </section>


      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="card-heading">
            <h3 className="card-title">{t('stats.trendTitle')}</h3>
          </div>
          {timeseriesError ? (
            <div className="alert alert-error">{timeseriesError}</div>
          ) : (
            <TimeseriesChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="card-heading">
            <h3 className="card-title">{t('stats.successFailureTitle')}</h3>
          </div>
          {timeseriesError ? (
            <div className="alert alert-error">{timeseriesError}</div>
          ) : (
            <SuccessFailureChart
              points={timeseries?.points ?? []}
              isLoading={timeseriesLoading}
              bucketSeconds={timeseries?.bucketSeconds}
            />
          )}
        </div>
      </section>

      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="card-heading">
            <h3 className="card-title">{t('stats.errors.title')}</h3>
          </div>
          {errorsError ? (
            <div className="alert alert-error">{errorsError}</div>
          ) : (
            <ErrorReasonPieChart items={errors?.items ?? []} isLoading={errorsLoading} />
          )}
        </div>
      </section>
    </div>
  )
}
