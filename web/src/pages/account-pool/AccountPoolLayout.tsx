import { NavLink, Navigate, Outlet, useLocation } from 'react-router-dom'
import { cn } from '../../lib/utils'
import { useTranslation } from '../../i18n'

const items = [{ to: '/account-pool/upstream-accounts', key: 'accountPool.nav.upstreamAccounts' }] as const

export default function AccountPoolLayout() {
  const { t } = useTranslation()
  const location = useLocation()

  if (location.pathname === '/account-pool') {
    return <Navigate to="/account-pool/upstream-accounts" replace />
  }

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
      <section className="surface-panel overflow-hidden">
        <div className="surface-panel-body gap-5">
          <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
            <div className="section-heading">
              <span className="text-xs font-semibold uppercase tracking-[0.24em] text-primary/80">
                {t('accountPool.eyebrow')}
              </span>
              <h1 className="section-title text-2xl sm:text-3xl">{t('accountPool.title')}</h1>
              <p className="section-description max-w-2xl">{t('accountPool.description')}</p>
            </div>
            <div className="segment-group self-start">
              {items.map((item) => (
                <NavLink
                  key={item.to}
                  to={item.to}
                  className={({ isActive }) => cn('segment-button', isActive && 'font-semibold')}
                  data-active={location.pathname.startsWith(item.to)}
                >
                  {t(item.key)}
                </NavLink>
              ))}
            </div>
          </div>
        </div>
      </section>
      <Outlet />
    </div>
  )
}
