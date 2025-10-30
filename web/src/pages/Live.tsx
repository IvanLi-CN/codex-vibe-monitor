import { useCallback, useState } from 'react'
import { InvocationChart } from '../components/InvocationChart'
import { InvocationTable } from '../components/InvocationTable'
import { StatsCards } from '../components/StatsCards'
import { useInvocationStream } from '../hooks/useInvocations'
import { useSummary } from '../hooks/useStats'

const LIMIT_OPTIONS = [20, 50, 100]
const SUMMARY_WINDOWS: { value: string; label: string }[] = [
  { value: 'current', label: '当前窗口' },
  { value: '30m', label: '30 分钟' },
  { value: '1h', label: '1 小时' },
  { value: '1d', label: '1 天' },
]

export default function LivePage() {
  const [limit, setLimit] = useState(50)
  const [summaryWindow, setSummaryWindow] = useState('current')

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
            <h2 className="card-title">实时统计</h2>
            <div className="join">
              {SUMMARY_WINDOWS.map((option) => (
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
            <h2 className="card-title">实时图表</h2>
            <label className="form-control w-36">
              <div className="label py-0">
                <span className="label-text text-xs uppercase tracking-wide">窗口大小</span>
              </div>
              <select
                className="select select-bordered select-sm"
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
              >
                {LIMIT_OPTIONS.map((value) => (
                  <option key={value} value={value}>
                    {value} 条记录
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
          <h2 className="card-title">最新记录</h2>
          <InvocationTable records={records} isLoading={isLoading} error={error} />
        </div>
      </section>
    </div>
  )
}
