import { NavLink, Navigate, Outlet, useLocation } from 'react-router-dom'
import { useTranslation } from '../../i18n'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { segmentedControlItemVariants } from '../../components/ui/segmented-control.variants'
import { accountPoolNavItems, matchesNavigationPath } from '../../features/app-shell/navigation'

export default function AccountPoolLayout() {
  const { t } = useTranslation()
  const location = useLocation()

  if (location.pathname === '/account-pool') {
    return <Navigate to="/account-pool/upstream-accounts" replace />
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
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
            <div className="hidden desktop:block">
              <SegmentedControl className="self-start">
                {accountPoolNavItems.map((item) => (
                  <NavLink
                    key={item.to}
                    to={item.to}
                    className={segmentedControlItemVariants({
                      active: matchesNavigationPath(location.pathname, item),
                    })}
                  >
                    {t(item.labelKey)}
                  </NavLink>
                ))}
              </SegmentedControl>
            </div>
          </div>
        </div>
      </section>
      <Outlet />
    </div>
  )
}
