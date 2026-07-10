import { useEffect, useMemo, useRef, useState } from 'react'
import { AppIcon } from '../shared/AppIcon'
import { NavLink, Outlet, useLocation } from 'react-router-dom'
import { subscribeToSse, requestImmediateReconnect } from '../../lib/sse'
import useSseStatus from '../../hooks/useSseStatus'
import useUpdateAvailable from '../../hooks/useUpdateAvailable'
import { fetchVersion } from '../../lib/api'
import type { VersionResponse } from '../../lib/api'
import { frontendVersion, normalizeVersion } from '../../lib/version'
import { useTranslation } from '../../i18n'
import { supportedLocales, type Locale } from '../../i18n'
import { useTheme } from '../../theme'
import { Button } from '../../components/ui/button'
import { SegmentedControl } from '../../components/ui/segmented-control'
import { segmentedControlItemVariants } from '../../components/ui/segmented-control.variants'
import { UpdateAvailableBanner } from './UpdateAvailableBanner'
import { HeaderBrandMark, type HeaderBrandMarkState } from './HeaderBrandMark'
import {
  desktopNavItems,
  matchesNavigationPath,
  mobileNavigationGroups,
  resolveAppNavigation,
} from './navigation'

const repositoryUrl = 'https://github.com/IvanLi-CN/codex-vibe-monitor'
const LOCALE_FLAG: Record<Locale, string> = {
  zh: '🇨🇳',
  en: '🇺🇸',
}
const OFFLINE_NOTICE_THRESHOLD_MS = 2 * 60 * 1000
export const HEADER_BRAND_ACTIVITY_HOLD_MS = 3200

export function AppLayout() {
  const location = useLocation()
  const { t, locale, setLocale } = useTranslation()
  const { themeMode, toggleTheme } = useTheme()
  const [hasRecentActivity, setHasRecentActivity] = useState(false)
  const activityTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const [versionInfo, setVersionInfo] = useState<VersionResponse | null>(null)
  const [backendLoading, setBackendLoading] = useState(true)
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
  const update = useUpdateAvailable()
  const sseStatus = useSseStatus()

  const isReconnecting = sseStatus.phase === 'connecting' || sseStatus.phase === 'reconnecting'
  const isSseDisabled = sseStatus.phase === 'disabled'
  const isOffline = sseStatus.phase !== 'connected' && sseStatus.phase !== 'idle'
  const showOfflineBanner = isOffline && sseStatus.downtimeMs >= OFFLINE_NOTICE_THRESHOLD_MS
  const downtimeSeconds = Math.max(Math.floor(sseStatus.downtimeMs / 1000), 0)
  const downtimeMinutesPart = Math.floor(downtimeSeconds / 60)
  const downtimeSecondsPart = downtimeSeconds % 60
  const nextRetrySeconds =
    sseStatus.nextRetryAt != null
      ? Math.max(Math.ceil((sseStatus.nextRetryAt - Date.now()) / 1000), 0)
      : null
  const durationChipLabel = t('app.sse.banner.durationChip', {
    minutes: downtimeMinutesPart,
    seconds: downtimeSecondsPart.toString().padStart(2, '0'),
  })
  const statusLine = sseStatus.autoReconnect
    ? nextRetrySeconds != null && nextRetrySeconds > 0
      ? t('app.sse.banner.retryIn', { seconds: nextRetrySeconds })
      : t('app.sse.banner.retryingNow')
    : t('app.sse.banner.autoDisabled')

  useEffect(() => {
    const clearActivityWindow = () => {
      if (activityTimeoutRef.current) {
        clearTimeout(activityTimeoutRef.current)
        activityTimeoutRef.current = null
      }
    }

    const unsubscribe = subscribeToSse(() => {
      if (sseStatus.phase !== 'connected') return
      setHasRecentActivity(true)
      clearActivityWindow()
      activityTimeoutRef.current = setTimeout(() => {
        activityTimeoutRef.current = null
        setHasRecentActivity(false)
      }, HEADER_BRAND_ACTIVITY_HOLD_MS)
    })
    return () => {
      clearActivityWindow()
      unsubscribe()
    }
  }, [sseStatus.phase])

  useEffect(() => {
    if (sseStatus.phase === 'connected') return
    if (activityTimeoutRef.current) {
      clearTimeout(activityTimeoutRef.current)
      activityTimeoutRef.current = null
    }
    setHasRecentActivity(false)
  }, [sseStatus.phase])

  useEffect(() => {
    setBackendLoading(true)
    fetchVersion()
      .then(setVersionInfo)
      .catch(() => setVersionInfo(null))
      .finally(() => setBackendLoading(false))
  }, [])

  const handleLocaleChange = (next: Locale) => {
    if (next !== locale) {
      setLocale(next)
    }
  }

  const [languageMenuOpen, setLanguageMenuOpen] = useState(false)
  const languageMenuRef = useRef<HTMLDivElement | null>(null)

  const localeChoices = useMemo(
    () =>
      supportedLocales.map((code) => ({
        code,
        flag: LOCALE_FLAG[code],
        label: t(code === 'zh' ? 'app.language.option.zh' : 'app.language.option.en'),
      })),
    [t],
  )

  const activeChoice = localeChoices.find((choice) => choice.code === locale) ?? localeChoices[0]
  const resolvedNavigation = useMemo(
    () => resolveAppNavigation(location.pathname),
    [location.pathname],
  )
  const mobileContextLabel = t(resolvedNavigation.nestedItem?.labelKey ?? resolvedNavigation.topLevelItem.labelKey)
  const mobileContextEyebrow = resolvedNavigation.nestedItem
    ? t(resolvedNavigation.topLevelItem.labelKey)
    : null

  const toggleLanguageMenu = () => {
    setLanguageMenuOpen((open) => !open)
  }

  const closeLanguageMenu = () => setLanguageMenuOpen(false)

  const handleManualReconnect = () => {
    requestImmediateReconnect()
  }

  useEffect(() => {
    if (!languageMenuOpen) return
    const handleClickOutside = (event: MouseEvent) => {
      if (!languageMenuRef.current?.contains(event.target as Node)) {
        closeLanguageMenu()
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeLanguageMenu()
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('mousedown', handleClickOutside)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [languageMenuOpen])

  useEffect(() => {
    setMobileNavOpen(false)
  }, [location.pathname, location.search])

  useEffect(() => {
    if (!mobileNavOpen || typeof document === 'undefined') return undefined

    const previousOverflow = document.body.style.overflow
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMobileNavOpen(false)
      }
    }

    document.body.style.overflow = 'hidden'
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.body.style.overflow = previousOverflow
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [mobileNavOpen])

  const normalizedFrontendVersion = normalizeVersion(frontendVersion)
  const normalizedBackendVersion = versionInfo?.backend ? normalizeVersion(versionInfo.backend) : null
  const releaseLink = normalizedBackendVersion
    ? `${repositoryUrl}/releases/tag/${normalizedBackendVersion}`
    : null

  const sameVersion =
    !!normalizedBackendVersion && normalizedBackendVersion === normalizedFrontendVersion

  function renderDiffVersion(oldV: string, newV: string) {
    // Simple, clear style: strike the whole old version (grey), arrow, then new version.
    return (
      <>
        <span>{t('app.footer.newVersionAvailable')}{' '}</span>
        <span className="font-mono text-base-content/60">
          <del style={{ textDecorationColor: 'currentColor' }}>{oldV}</del>
        </span>
        <span aria-hidden>{' '}→{' '}</span>
        <a
          className="app-link font-mono"
          href={releaseLink ?? undefined}
          target="_blank"
          rel="noreferrer"
        >
          {newV}
        </a>
      </>
    )
  }

  const isDarkTheme = themeMode === 'dark'
  const themeLabel = t(isDarkTheme ? 'app.theme.currentDark' : 'app.theme.currentLight')
  const themeSwitcherLabel = t(isDarkTheme ? 'app.theme.switchToLight' : 'app.theme.switchToDark')
  const headerBrandMarkState: HeaderBrandMarkState = isSseDisabled
    ? 'disabled'
    : isReconnecting
      ? 'reconnecting'
      : hasRecentActivity
        ? 'active'
        : 'idle'

  return (
    <div className="app-shell min-h-screen flex flex-col text-base-content">
      <header className="sticky top-0 z-50 border-b border-base-300/75 bg-base-100/80 backdrop-blur-md">
        <div className="app-shell-boundary flex items-center gap-2 px-3 py-2 sm:px-4" data-testid="app-header-inner">
          <div className="flex min-w-0 flex-1 items-center gap-2.5 sm:gap-3">
            <button
              type="button"
              className="control-pill min-[1024px]:hidden"
              onClick={() => setMobileNavOpen(true)}
              aria-label={t('app.nav.openMenu')}
              aria-expanded={mobileNavOpen}
              aria-controls="app-mobile-navigation"
            >
              <AppIcon name="navigation-variant" className="h-[18px] w-[18px] text-primary" aria-hidden />
              <span className="sr-only">{t('app.nav.openMenu')}</span>
            </button>
            <HeaderBrandMark
              alt={t('app.logoAlt')}
              state={headerBrandMarkState}
              className="h-9 w-9 min-[1024px]:h-10 min-[1024px]:w-10"
              markClassName="h-9 w-9 min-[1024px]:h-10 min-[1024px]:w-10"
              data-testid="app-header-logo-mark"
            />
            <div className="min-w-0">
              <span className="hidden truncate text-lg font-semibold tracking-tight min-[1024px]:block min-[1024px]:text-xl">
                {t('app.brand')}
              </span>
              <div className="min-[1024px]:hidden">
                {mobileContextEyebrow ? (
                  <p className="truncate text-[10px] font-semibold uppercase tracking-[0.18em] text-primary/72">
                    {mobileContextEyebrow}
                  </p>
                ) : null}
                <p className="truncate text-sm font-semibold tracking-tight">{mobileContextLabel}</p>
              </div>
            </div>
          </div>

          <nav className="flex shrink-0 items-center gap-2 sm:gap-3">
            <div className="hidden overflow-x-auto no-scrollbar min-[1024px]:block">
              <SegmentedControl size="nav" className="min-w-max" aria-label={t('app.brand')}>
                {desktopNavItems.map((item) => (
                  <NavLink
                    key={item.to}
                    to={item.to}
                    aria-current={matchesNavigationPath(location.pathname, item) ? 'page' : undefined}
                    className={segmentedControlItemVariants({
                      size: 'nav',
                      active: matchesNavigationPath(location.pathname, item),
                    })}
                  >
                    {t(item.labelKey)}
                  </NavLink>
                ))}
              </SegmentedControl>
            </div>

            <button
              type="button"
              className="control-pill"
              onClick={toggleTheme}
              aria-label={t('app.theme.switcherAria')}
              title={themeSwitcherLabel}
            >
              <AppIcon
                name={isDarkTheme ? 'weather-night' : 'white-balance-sunny'}
                className="h-[18px] w-[18px] text-primary"
                aria-hidden
              />
              <span className="hidden md:inline">{themeLabel}</span>
            </button>

            <div
              ref={languageMenuRef}
              className="relative"
            >
              <button
                type="button"
                className="control-pill min-w-0 justify-between sm:min-w-[6.75rem]"
                aria-haspopup="listbox"
                aria-expanded={languageMenuOpen}
                aria-label={t('app.language.switcherAria')}
                onClick={toggleLanguageMenu}
              >
                <AppIcon name="earth" className="h-[18px] w-[18px] text-base-content/75" aria-hidden />
                <span className="hidden sm:inline">{activeChoice?.label}</span>
                <AppIcon name="chevron-down" className="h-4 w-4 text-base-content/60" aria-hidden />
              </button>
              <ul
                className={`absolute right-0 top-[calc(100%+0.4rem)] z-50 mt-2 min-w-[10.5rem] rounded-xl border border-base-300 bg-base-100/95 p-2 shadow-lg backdrop-blur ${
                  languageMenuOpen ? 'block' : 'hidden'
                }`}
                role="listbox"
                aria-label={t('app.language.switcherAria')}
              >
                {localeChoices.map((choice) => (
                  <li key={choice.code} role="presentation">
                    <button
                      type="button"
                      className={`flex items-center gap-2 rounded-md px-2 py-1 ${
                        choice.code === locale ? 'bg-primary/15 font-medium text-primary' : 'hover:bg-base-200'
                      }`}
                      onClick={() => {
                        handleLocaleChange(choice.code)
                        closeLanguageMenu()
                      }}
                      role="option"
                      aria-selected={choice.code === locale}
                    >
                      <span aria-hidden>{choice.flag}</span>
                      <span>{choice.label}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          </div>

        </div>
      </header>
      {mobileNavOpen ? (
        <div className="fixed inset-0 z-[85] min-[1024px]:hidden">
          <button
            type="button"
            aria-label={t('app.nav.closeMenu')}
            className="absolute inset-0 bg-neutral/56 backdrop-blur-sm"
            onClick={() => setMobileNavOpen(false)}
          />
          <aside
            id="app-mobile-navigation"
            className="absolute inset-y-0 left-0 flex w-[min(22rem,calc(100vw-1rem))] max-w-full flex-col border-r border-base-300/75 bg-base-100/96 px-4 pb-[max(env(safe-area-inset-bottom),1rem)] pt-[max(env(safe-area-inset-top),1rem)] shadow-[0_24px_72px_rgba(15,23,42,0.2)] backdrop-blur-xl"
          >
            <div className="flex items-start justify-between gap-3 border-b border-base-300/70 pb-4">
              <div className="flex min-w-0 items-center gap-3">
                <HeaderBrandMark
                  alt={t('app.logoAlt')}
                  state={headerBrandMarkState}
                  className="h-9 w-9"
                  markClassName="h-9 w-9"
                />
                <div className="min-w-0">
                  <p className="truncate text-sm font-semibold tracking-tight">{t('app.brand')}</p>
                  <p className="truncate text-xs text-base-content/62">{mobileContextLabel}</p>
                </div>
              </div>
              <button
                type="button"
                className="control-pill"
                onClick={() => setMobileNavOpen(false)}
                aria-label={t('app.nav.closeMenu')}
              >
                <AppIcon name="close" className="h-[18px] w-[18px] text-base-content/78" aria-hidden />
                <span className="sr-only">{t('app.nav.closeMenu')}</span>
              </button>
            </div>

            <nav className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto py-4 pr-1">
              <div className="flex flex-col gap-1.5">
                {mobileNavigationGroups
                  .filter((group) => group.items.length === 0)
                  .map((item) => (
                    <NavLink
                      key={item.to}
                      to={item.to}
                      className={[
                        'flex items-center justify-between rounded-2xl border px-4 py-3 text-left text-sm font-medium transition-colors',
                        matchesNavigationPath(location.pathname, item)
                          ? 'border-primary/45 bg-primary/12 text-primary'
                          : 'border-base-300/70 bg-base-100/72 text-base-content/78 hover:border-primary/30 hover:text-base-content',
                      ].join(' ')}
                    >
                      <span>{t(item.labelKey)}</span>
                      <AppIcon name="chevron-right" className="h-4 w-4" aria-hidden />
                    </NavLink>
                  ))}
              </div>

              {mobileNavigationGroups
                .filter((group) => group.items.length > 0)
                .map((group) => (
                  <section key={group.to} className="space-y-2">
                    <p className="px-1 text-[11px] font-semibold uppercase tracking-[0.18em] text-base-content/52">
                      {t(group.labelKey)}
                    </p>
                    <div className="flex flex-col gap-1.5">
                      {group.items.map((item) => (
                        <NavLink
                          key={item.to}
                          to={item.to}
                          className={[
                            'rounded-2xl border px-4 py-3 text-sm font-medium transition-colors',
                            matchesNavigationPath(location.pathname, item)
                              ? 'border-primary/45 bg-primary/12 text-primary'
                              : 'border-base-300/70 bg-base-100/72 text-base-content/78 hover:border-primary/30 hover:text-base-content',
                          ].join(' ')}
                        >
                          {t(item.labelKey)}
                        </NavLink>
                      ))}
                    </div>
                  </section>
                ))}
            </nav>
          </aside>
        </div>
      ) : null}
      {showOfflineBanner && (
        <div className="fixed left-1/2 top-[78px] z-[60] w-full max-w-3xl -translate-x-1/2 px-4">
          <div
            className="flex w-full flex-col gap-3 rounded-xl border border-warning/60 bg-warning/90 p-4 text-warning-content shadow-lg sm:flex-row sm:items-center"
            role="status"
            aria-live="assertive"
          >
            <div className="flex min-w-0 flex-1 items-center gap-3">
              <AppIcon name="alert-circle" className="h-6 w-6 flex-shrink-0" aria-hidden />
              <div className="min-w-0 space-y-1">
                <div className="flex flex-wrap items-center gap-3">
                  <span className="font-semibold">{t('app.sse.banner.title')}</span>
                  <span className="rounded-full bg-warning/20 px-2 py-0.5 text-xs font-mono text-warning-content">
                    {durationChipLabel}
                  </span>
                </div>
                <p className="text-sm text-warning-content/90 truncate">
                  {t('app.sse.banner.description')} · {statusLine}
                </p>
              </div>
            </div>
            <Button
              type="button"
              size="sm"
              className="w-full sm:ml-auto sm:w-auto"
              onClick={handleManualReconnect}
            >
              {t('app.sse.banner.reconnectButton')}
            </Button>
          </div>
        </div>
      )}
      {update.visible && update.availableVersion && (
        <UpdateAvailableBanner
          currentVersion={versionInfo?.backend ?? t('app.update.current')}
          availableVersion={update.availableVersion}
          onReload={update.reload}
          onDismiss={update.dismiss}
          labels={{
            available: t('app.update.available'),
            refresh: t('app.update.refresh'),
            later: t('app.update.later'),
          }}
        />
      )}
      <main
        className="app-shell-boundary flex-1 min-h-0 px-3 py-5 pb-8 sm:px-4 sm:py-6"
        data-testid="app-main"
      >
        <Outlet />
      </main>
      <footer
        className="border-t border-base-300/75 bg-base-100/80 text-sm text-base-content/70 backdrop-blur"
        data-testid="app-footer"
      >
        <div
          className="app-shell-boundary flex flex-wrap items-center justify-between gap-3 px-4 py-3"
          data-testid="app-footer-inner"
        >
          <span>{t('app.footer.copyright')}</span>
          <div className="flex flex-wrap items-center gap-4">
            <a
              className="app-link flex items-center gap-1"
              href={repositoryUrl}
              target="_blank"
              rel="noreferrer"
              aria-label={t('app.footer.githubAria')}
            >
              <AppIcon name="github" className="h-4 w-4" aria-hidden />
              <span>GitHub</span>
            </a>
            <div className="flex items-center gap-2">
              {sameVersion && normalizedBackendVersion ? (
                releaseLink ? (
                  <a
                    className="app-link font-mono"
                    href={releaseLink}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {normalizedBackendVersion}
                  </a>
                ) : (
                  <span className="font-mono">{normalizedFrontendVersion}</span>
                )
              ) : normalizedBackendVersion ? (
                <span className="inline-flex items-center gap-2">
                  {renderDiffVersion(normalizedFrontendVersion, normalizedBackendVersion)}
                  {backendLoading && (
                    <span className="flex items-center gap-1 text-base-content/60" aria-live="polite">
                      <AppIcon name="loading" className="h-3 w-3 animate-spin" aria-hidden />
                      <span className="sr-only">{t('app.footer.loadingVersion')}</span>
                    </span>
                  )}
                </span>
              ) : (
                <span className="inline-flex items-center gap-2">
                  <span className="font-mono">{normalizedFrontendVersion}</span>
                  {backendLoading && (
                    <span className="flex items-center gap-1 text-base-content/60" aria-live="polite">
                      <AppIcon name="loading" className="h-3 w-3 animate-spin" aria-hidden />
                      <span className="sr-only">{t('app.footer.loadingVersion')}</span>
                    </span>
                  )}
                </span>
              )}
            </div>
          </div>
        </div>
      </footer>
    </div>
  )
}
