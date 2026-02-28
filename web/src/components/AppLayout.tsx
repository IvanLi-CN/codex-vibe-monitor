import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { NavLink, Outlet } from 'react-router-dom'
import { subscribeToSse, requestImmediateReconnect } from '../lib/sse'
import useSseStatus from '../hooks/useSseStatus'
import useUpdateAvailable from '../hooks/useUpdateAvailable'
import { fetchVersion } from '../lib/api'
import type { VersionResponse } from '../lib/api'
import { frontendVersion, normalizeVersion } from '../lib/version'
import { useTranslation } from '../i18n'
import { supportedLocales, type Locale } from '../i18n'
import { useTheme } from '../theme'
import { Button } from './ui/button'
import { UpdateAvailableBanner } from './UpdateAvailableBanner'
import { cn } from '../lib/utils'

const navItems = [
  { to: '/dashboard', labelKey: 'app.nav.dashboard' },
  { to: '/stats', labelKey: 'app.nav.stats' },
  { to: '/live', labelKey: 'app.nav.live' },
  { to: '/settings', labelKey: 'app.nav.settings' },
] as const

const repositoryUrl = 'https://github.com/IvanLi-CN/codex-vibe-monitor'
const LOCALE_FLAG: Record<Locale, string> = {
  zh: '🇨🇳',
  en: '🇺🇸',
}
const OFFLINE_NOTICE_THRESHOLD_MS = 2 * 60 * 1000

export function AppLayout() {
  const { t, locale, setLocale } = useTranslation()
  const { themeMode, toggleTheme } = useTheme()
  const [pulse, setPulse] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const animationDurationMs = 1400
  const [versionInfo, setVersionInfo] = useState<VersionResponse | null>(null)
  const [backendLoading, setBackendLoading] = useState(true)
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
    const unsubscribe = subscribeToSse(() => {
      setPulse(true)
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current)
      }
      timeoutRef.current = setTimeout(() => setPulse(false), animationDurationMs)
    })
    setBackendLoading(true)
    fetchVersion()
      .then(setVersionInfo)
      .catch(() => setVersionInfo(null))
      .finally(() => setBackendLoading(false))
    return () => {
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current)
      }
      unsubscribe()
    }
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

  const logoImageClass = `h-8 w-8 relative z-20 transition-transform duration-300 ${
    pulse
      ? 'animate-pulse-core scale-110 drop-shadow-[0_0_18px_rgba(59,130,246,0.65)]'
      : 'drop-shadow-[0_0_6px_rgba(59,130,246,0.35)]'
  } ${isOffline ? 'grayscale opacity-70' : ''} ${isSseDisabled ? 'opacity-60' : ''}`

  const reconnectRingClass = `pointer-events-none absolute inline-flex h-14 w-14 rounded-full border-2 border-dashed transition-opacity duration-300 ${
    isSseDisabled ? 'border-warning/80' : 'border-primary/70'
  } ${isReconnecting ? 'opacity-95 animate-orbit-spin' : 'opacity-0'}`

  const isDarkTheme = themeMode === 'dark'
  const themeLabel = t(isDarkTheme ? 'app.theme.currentDark' : 'app.theme.currentLight')
  const themeSwitcherLabel = t(isDarkTheme ? 'app.theme.switchToLight' : 'app.theme.switchToDark')

  return (
    <div className="app-shell min-h-screen flex flex-col text-base-content">
      <header className="sticky top-0 z-50 border-b border-base-300/75 bg-base-100/80 backdrop-blur-md">
        <div className="mx-auto flex w-full max-w-[1200px] items-center gap-2 px-4 py-2">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span className="relative inline-flex items-center justify-center">
              <span
                className={`pointer-events-none absolute inline-flex h-16 w-16 rounded-full bg-gradient-to-r from-primary/30 via-primary/5 to-primary/30 opacity-0 transition-opacity ${
                  pulse ? 'opacity-95 animate-pulse-glow' : ''
                }`}
                aria-hidden
              />
              <span className={reconnectRingClass} aria-hidden />
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
              <img src="/favicon.svg" alt={t('app.logoAlt')} className={logoImageClass} />
            </span>
            <span className="truncate text-lg font-semibold tracking-tight sm:text-xl">{t('app.brand')}</span>
          </div>

          <nav className="flex shrink-0 items-center gap-2 sm:gap-3">
            <div className="max-w-[40vw] overflow-x-auto no-scrollbar sm:max-w-none">
              <ul className="segment-group">
                {navItems.map((item) => (
                  <li key={item.to}>
                    <NavLink
                      to={item.to}
                      className={({ isActive }) =>
                        cn('app-nav-item', isActive && 'app-nav-item-active')
                      }
                    >
                      {t(item.labelKey)}
                    </NavLink>
                  </li>
                ))}
              </ul>
            </div>

            <button
              type="button"
              className="control-pill"
              onClick={toggleTheme}
              aria-label={t('app.theme.switcherAria')}
              title={themeSwitcherLabel}
            >
              <Icon
                icon={isDarkTheme ? 'mdi:weather-night' : 'mdi:white-balance-sunny'}
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
                className="control-pill min-w-[6.75rem] justify-between"
                aria-haspopup="listbox"
                aria-expanded={languageMenuOpen}
                aria-label={t('app.language.switcherAria')}
                onClick={toggleLanguageMenu}
              >
                <Icon icon="mdi:earth" className="h-[18px] w-[18px] text-base-content/75" aria-hidden />
                <span className="hidden sm:inline">{activeChoice?.label}</span>
                <Icon icon="mdi:chevron-down" className="h-4 w-4 text-base-content/60" aria-hidden />
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
          </nav>
        </div>
      </header>
      {showOfflineBanner && (
        <div className="fixed left-1/2 top-[78px] z-[60] w-full max-w-3xl -translate-x-1/2 px-4">
          <div
            className="flex w-full flex-col gap-3 rounded-xl border border-warning/60 bg-warning/90 p-4 text-warning-content shadow-lg sm:flex-row sm:items-center"
            role="status"
            aria-live="assertive"
          >
            <div className="flex min-w-0 flex-1 items-center gap-3">
              <Icon icon="mdi:alert-circle" className="h-6 w-6 flex-shrink-0" aria-hidden />
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
      <main className="mx-auto w-full max-w-[1200px] flex-1 min-h-0 px-4 py-6 pb-8">
        <Outlet />
      </main>
      <footer
        className="border-t border-base-300/75 bg-base-100/80 text-sm text-base-content/70 backdrop-blur"
        data-testid="app-footer"
      >
        <div className="mx-auto flex w-full max-w-[1200px] flex-wrap items-center justify-between gap-3 px-4 py-3">
          <span>{t('app.footer.copyright')}</span>
          <div className="flex flex-wrap items-center gap-4">
            <a
              className="app-link flex items-center gap-1"
              href={repositoryUrl}
              target="_blank"
              rel="noreferrer"
              aria-label={t('app.footer.githubAria')}
            >
              <Icon icon="mdi:github" className="h-4 w-4" aria-hidden />
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
                      <Icon icon="mdi:loading" className="h-3 w-3 animate-spin" aria-hidden />
                      <span className="sr-only">{t('app.footer.loadingVersion')}</span>
                    </span>
                  )}
                </span>
              ) : (
                <span className="inline-flex items-center gap-2">
                  <span className="font-mono">{normalizedFrontendVersion}</span>
                  {backendLoading && (
                    <span className="flex items-center gap-1 text-base-content/60" aria-live="polite">
                      <Icon icon="mdi:loading" className="h-3 w-3 animate-spin" aria-hidden />
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
