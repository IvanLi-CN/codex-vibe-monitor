import { NavLink, Navigate, Outlet, useLocation } from 'react-router-dom'
import { useTranslation } from '../../i18n'
import { useCompactViewport } from '../../hooks/useCompactViewport'
import { cn } from '../../lib/utils'
import { matchesNavigationPath, systemNavItems } from '../../features/app-shell/navigation'

export default function SystemLayout() {
  const { t } = useTranslation()
  const location = useLocation()
  const isCompactViewport = useCompactViewport()

  if (location.pathname === '/system') {
    return <Navigate to="/system/status" replace />
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="section-heading">
            <span className="text-xs font-semibold uppercase tracking-[0.24em] text-primary/80">
              {t('system.eyebrow')}
            </span>
            <h1 className="section-title text-2xl sm:text-3xl">{t('system.title')}</h1>
            <p className="section-description max-w-3xl">{t('system.description')}</p>
          </div>
        </div>
      </section>

      {isCompactViewport ? (
        <Outlet />
      ) : (
        <div className="grid gap-6 lg:grid-cols-[15rem_minmax(0,1fr)] lg:items-start">
          <aside className="surface-panel overflow-hidden">
            <div className="surface-panel-body gap-3">
              <div className="text-xs font-semibold uppercase tracking-[0.18em] text-base-content/55">
                {t('system.nav.label')}
              </div>
              <nav className="-mx-2 flex gap-2 overflow-x-auto px-2 no-scrollbar lg:mx-0 lg:flex-col lg:overflow-visible lg:px-0">
                {systemNavItems.map((item) => {
                  const active = matchesNavigationPath(location.pathname, item)
                  return (
                    <NavLink
                      key={item.to}
                      to={item.to}
                      className={cn(
                        'min-w-max rounded-xl border px-3.5 py-3 text-sm transition-colors lg:min-w-0',
                        active
                          ? 'border-primary/45 bg-primary/10 text-primary'
                          : 'border-base-300/70 bg-base-100/72 text-base-content/78 hover:border-primary/30 hover:text-base-content',
                      )}
                    >
                      {t(item.labelKey)}
                    </NavLink>
                  )
                })}
              </nav>
            </div>
          </aside>

          <Outlet />
        </div>
      )}
    </div>
  )
}
