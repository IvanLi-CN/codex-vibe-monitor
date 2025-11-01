import { useEffect, useRef, useState } from 'react'
import { NavLink, Outlet } from 'react-router-dom'
import { subscribeToSse } from '../lib/sse'
import { fetchVersion } from '../lib/api'
import type { VersionResponse } from '../lib/api'

const navItems = [
  { to: '/dashboard', label: 'Dashboard' },
  { to: '/stats', label: '统计' },
  { to: '/live', label: '实况' },
]

export function AppLayout() {
  const [pulse, setPulse] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const animationDurationMs = 1400
  const [version, setVersion] = useState<VersionResponse | null>(null)

  useEffect(() => {
    const unsubscribe = subscribeToSse(() => {
      setPulse(true)
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current)
      }
      timeoutRef.current = setTimeout(() => setPulse(false), animationDurationMs)
    })
    fetchVersion().then(setVersion).catch(() => setVersion(null))
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current)
      }
      unsubscribe()
    }
  }, [])

  return (
    <div className="min-h-screen bg-base-200 text-base-content">
      <header className="navbar bg-base-100 border-b border-base-300 sticky top-0 z-50">
        <div className="flex flex-1 items-center gap-2 px-4">
          <span className="relative inline-flex items-center justify-center">
            <span
              className={`pointer-events-none absolute inline-flex h-16 w-16 rounded-full bg-gradient-to-r from-primary/30 via-primary/5 to-primary/30 opacity-0 transition-opacity ${
                pulse ? 'opacity-95 animate-pulse-glow' : ''
              }`}
              aria-hidden
            />
            <span
              className={`pointer-events-none absolute inline-flex h-12 w-12 rounded-full border-2 border-primary/70 transition-opacity ${
                pulse ? 'opacity-100 animate-pulse-ring' : 'opacity-0'
              }`}
              aria-hidden
            />
            <span
              className={`pointer-events-none absolute inline-flex h-10 w-10 rounded-full bg-primary/30 blur-md transition-opacity ${
                pulse ? 'opacity-80' : 'opacity-0'
              }`}
              aria-hidden
            />
            <img
              src="/favicon.svg"
              alt="Codex Vibe Monitor icon"
              className={`h-8 w-8 relative z-20 transition-transform duration-300 ${
                pulse
                  ? 'animate-pulse-core scale-110 drop-shadow-[0_0_18px_rgba(59,130,246,0.65)]'
                  : 'drop-shadow-[0_0_6px_rgba(59,130,246,0.35)]'
              }`}
            />
          </span>
          <span className="text-xl font-semibold">Codex Vibe Monitor</span>
        </div>
        <nav className="flex-none">
          <ul className="menu menu-horizontal px-1">
            {navItems.map((item) => (
              <li key={item.to}>
                <NavLink
                  to={item.to}
                  className={({ isActive }) =>
                    isActive ? 'active font-semibold text-primary' : 'font-medium'
                  }
                >
                  {item.label}
                </NavLink>
              </li>
            ))}
          </ul>
        </nav>
      </header>
      <main className="px-4 py-6 pb-16">
        <Outlet />
      </main>
      <footer className="bt border-base-300 bg-base-100 text-sm text-base-content/70 w-full py-2 px-4 fixed bottom-0 left-0 flex items-center justify-between">
        <span>© Codex Vibe Monitor</span>
        <span>
          {version ? (
            <>Version v{version.backend}</>
          ) : (
            'Loading version…'
          )}
        </span>
      </footer>
    </div>
  )
}
