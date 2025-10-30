import { useCallback, useMemo, useState } from 'react'
import { NavLink, Navigate, Route, Routes, useLocation } from 'react-router-dom'
import { InvocationChart } from './components/InvocationChart'
import { InvocationTable } from './components/InvocationTable'
import { StatsCards } from './components/StatsCards'
import { useInvocationStream } from './hooks/useInvocations'
import { useStats } from './hooks/useStats'

const LIMIT_OPTIONS = [20, 50, 100]

function Tabs() {
  const location = useLocation()
  const items = useMemo(
    () => [
      { to: '/chart', label: 'Chart View' },
      { to: '/list', label: 'List View' },
    ],
    [],
  )

  return (
    <div role="tablist" className="tabs tabs-boxed">
      {items.map((item) => (
        <NavLink
          key={item.to}
          to={item.to}
          role="tab"
          className={({ isActive }) =>
            `tab ${
              isActive || location.pathname === '/' && item.to === '/chart'
                ? 'tab-active'
                : ''
            }`
          }
        >
          {item.label}
        </NavLink>
      ))}
    </div>
  )
}

function App() {
  const [limit, setLimit] = useState<number>(50)
  const {
    stats,
    isLoading: statsLoading,
    error: statsError,
    refresh: refreshStats,
  } = useStats()

  const handleNewRecords = useCallback(() => {
    void refreshStats()
  }, [refreshStats])

  const { records, isLoading, error } = useInvocationStream(limit, undefined, handleNewRecords)

  return (
    <div className="min-h-full bg-base-200">
      <header className="navbar bg-base-100 border-b border-base-300">
        <div className="flex-1 px-2">
          <div className="flex items-center gap-3">
            <img
              src="/favicon.svg"
              alt="Codex Vibe Monitor icon"
              className="h-8 w-8"
            />
            <span className="text-xl font-semibold">Codex Vibe Monitor</span>
          </div>
        </div>
        <div className="flex-none gap-4 px-2">
          <label className="form-control w-36">
            <div className="label py-0">
              <span className="label-text text-xs uppercase tracking-wide">Recent window</span>
            </div>
            <select
              className="select select-bordered select-sm"
              value={limit}
              onChange={(event) => setLimit(Number(event.target.value))}
            >
              {LIMIT_OPTIONS.map((value) => (
                <option key={value} value={value}>
                  {value} rows
                </option>
              ))}
            </select>
          </label>
        </div>
      </header>

      <main className="max-w-6xl mx-auto flex flex-col gap-6 p-6">
        <StatsCards stats={stats} loading={statsLoading} error={statsError} />

        <div className="flex items-center justify-between">
          <Tabs />
        </div>

        <div className="card bg-base-100 shadow-sm">
          <div className="card-body gap-6">
            <Routes>
              <Route
                path="/chart"
                element={<InvocationChart records={records} isLoading={isLoading} />}
              />
              <Route
                path="/list"
                element={<InvocationTable records={records} isLoading={isLoading} error={error} />}
              />
              <Route path="/" element={<Navigate to="/chart" replace />} />
              <Route path="*" element={<Navigate to="/chart" replace />} />
            </Routes>
          </div>
        </div>
      </main>
    </div>
  )
}

export default App
