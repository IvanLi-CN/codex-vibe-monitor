import { useEffect, useMemo, useState } from 'react'
import { StatsCards } from '../components/StatsCards'
import { TimeseriesChart } from '../components/TimeseriesChart'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'

const RANGE_OPTIONS = [
  { value: '1h', label: '最近 1 小时' },
  { value: '1d', label: '最近 1 天' },
  { value: '1mo', label: '最近 1 个月' },
] as const

const BUCKET_OPTIONS: Record<string, { value: string; label: string }[]> = {
  '1h': [
    { value: '1m', label: '每分钟' },
    { value: '5m', label: '每 5 分钟' },
    { value: '15m', label: '每 15 分钟' },
  ],
  '1d': [
    { value: '15m', label: '每 15 分钟' },
    { value: '30m', label: '每 30 分钟' },
    { value: '1h', label: '每小时' },
    { value: '6h', label: '每 6 小时' },
  ],
  '1mo': [
    { value: '6h', label: '每 6 小时' },
    { value: '12h', label: '每 12 小时' },
    { value: '1d', label: '每天' },
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
  const [range, setRange] = useState<typeof RANGE_OPTIONS[number]['value']>('1d')
  const bucketOptions = useMemo(() => BUCKET_OPTIONS[range] ?? BUCKET_OPTIONS['1d'], [range])
  const [bucket, setBucket] = useState<string>(bucketOptions[0]?.value ?? '1h')
  const [settlementHour, setSettlementHour] = useState(0)

  useEffect(() => {
    const nextBucket = bucketOptions[0]?.value
    if (nextBucket && !bucketOptions.some((option) => option.value === bucket)) {
      setBucket(nextBucket)
    }
  }, [bucket, bucketOptions])

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
              <h2 className="card-title">统计</h2>
              <p className="text-sm text-base-content/60">选择时间范围与聚合粒度</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              <select
                className="select select-bordered select-sm"
                value={range}
                onChange={(event) => setRange(event.target.value as typeof range)}
              >
                {RANGE_OPTIONS.map((option) => (
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
                    <span className="label-text text-xs">结算小时</span>
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
          <h3 className="card-title">趋势</h3>
          {timeseriesError ? (
            <div className="alert alert-error">{timeseriesError}</div>
          ) : (
            <TimeseriesChart points={timeseries?.points ?? []} isLoading={timeseriesLoading} />
          )}
        </div>
      </section>
    </div>
  )
}
