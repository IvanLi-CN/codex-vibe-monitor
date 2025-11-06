import { useEffect, useMemo, useState } from 'react'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'

const RANGE_OPTIONS = [
  { value: '1h', labelKey: 'stats.range.lastHour' },
  { value: '1d', labelKey: 'stats.range.lastDay' },
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
  '1mo': [
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
  const [range, setRange] = useState<typeof RANGE_OPTIONS[number]['value']>('1d')
  const rawBucketOptions = useMemo(() => BUCKET_OPTION_KEYS[range] ?? BUCKET_OPTION_KEYS['1d'], [range])
  const [bucket, setBucket] = useState<string>(rawBucketOptions[0]?.value ?? '1h')
  const [settlementHour, setSettlementHour] = useState(0)

  useEffect(() => {
    const nextBucket = rawBucketOptions[0]?.value
    if (nextBucket && !rawBucketOptions.some((option) => option.value === bucket)) {
      setBucket(nextBucket)
    }
  }, [bucket, rawBucketOptions])

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const bucketOptions = useMemo(
    () => rawBucketOptions.map((option) => ({ ...option, label: t(option.labelKey) })),
    [rawBucketOptions, t],
  )

  const needsSettlement = BUCKET_SECONDS[bucket] >= 86_400

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
    bucket,
    settlementHour: needsSettlement ? settlementHour : undefined,
  })

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <section className="card bg-base-100 shadow-sm">
        <div className="card-body gap-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <h2 className="card-title">{t('stats.title')}</h2>
              <p className="text-sm text-base-content/60">{t('stats.subtitle')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <select
                className="select select-bordered select-sm"
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
                className="select select-bordered select-sm"
                value={bucket}
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
          <h3 className="card-title">{t('stats.trendTitle')}</h3>
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
    </div>
  )
}
