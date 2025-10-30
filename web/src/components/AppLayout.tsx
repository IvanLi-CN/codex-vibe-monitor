import { NavLink, Outlet } from 'react-router-dom'

const navItems = [
  { to: '/dashboard', label: 'Dashboard' },
  { to: '/stats', label: '统计' },
  { to: '/live', label: '实况' },
]

export function AppLayout() {
  return (
    <div className="min-h-screen bg-base-200 text-base-content">
      <header className="navbar bg-base-100 border-b border-base-300 sticky top-0 z-50">
        <div className="flex flex-1 items-center gap-2 px-4">
          <img src="/favicon.svg" alt="Codex Vibe Monitor icon" className="h-8 w-8" />
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
      <main className="px-4 py-6">
        <Outlet />
      </main>
    </div>
  )
}
