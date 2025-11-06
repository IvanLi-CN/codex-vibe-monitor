import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { NavLink, Outlet } from 'react-router-dom'
import { subscribeToSse } from '../lib/sse'
import useUpdateAvailable from '../hooks/useUpdateAvailable'
import { fetchVersion } from '../lib/api'
import type { VersionResponse } from '../lib/api'
import { useTranslation } from '../i18n'
import { supportedLocales, type Locale } from '../i18n'

const navItems = [
  { to: '/dashboard', labelKey: 'app.nav.dashboard' },
  { to: '/stats', labelKey: 'app.nav.stats' },
  { to: '/live', labelKey: 'app.nav.live' },
] as const

const repositoryUrl = 'https://github.com/IvanLi-CN/codex-vibe-monitor'
const LOCALE_FLAG: Record<Locale, string> = {
  zh: '🇨🇳',
  en: '🇺🇸',
}

export function AppLayout() {
  const { t, locale, setLocale } = useTranslation()
  const [pulse, setPulse] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const animationDurationMs = 1400
  const [version, setVersion] = useState<VersionResponse | null>(null)
  const update = useUpdateAvailable()
  const tagVersion = version?.version.startsWith('v') ? version.version : (version ? `v${version.version}` : null)

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
              alt={t('app.logoAlt')}
              className={`h-8 w-8 relative z-20 transition-transform duration-300 ${
                pulse
                  ? 'animate-pulse-core scale-110 drop-shadow-[0_0_18px_rgba(59,130,246,0.65)]'
                  : 'drop-shadow-[0_0_6px_rgba(59,130,246,0.35)]'
              }`}
            />
          </span>
          <span className="text-xl font-semibold">{t('app.brand')}</span>
        </div>
        <nav className="flex-none flex items-center gap-4">
          <ul className="menu menu-horizontal px-1">
            {navItems.map((item) => (
              <li key={item.to}>
                <NavLink
                  to={item.to}
                  className={({ isActive }) =>
                    isActive ? 'active font-semibold text-primary' : 'font-medium'
                  }
                >
                  {t(item.labelKey)}
                </NavLink>
              </li>
            ))}
          </ul>
          <div className="flex items-center">
            <div
              ref={languageMenuRef}
              className={`dropdown dropdown-end dropdown-bottom ${languageMenuOpen ? 'dropdown-open' : ''}`}
            >
              <button
                type="button"
                className="btn btn-sm btn-ghost gap-2"
                aria-haspopup="listbox"
                aria-expanded={languageMenuOpen}
                aria-label={t('app.language.switcherAria')}
                onClick={toggleLanguageMenu}
              >
                <Icon icon="mdi:earth" className="h-5 w-5 text-base-content/70" aria-hidden />
                <span>{activeChoice?.label}</span>
                <Icon icon="mdi:chevron-down" className="h-4 w-4 text-base-content/60" aria-hidden />
              </button>
              <ul
                className="dropdown-content menu menu-sm rounded-box bg-base-100 p-2 mt-2 shadow border border-base-200"
                role="listbox"
                aria-label={t('app.language.switcherAria')}
              >
                {localeChoices.map((choice) => (
                  <li key={choice.code} role="presentation">
                    <button
                      type="button"
                      className={`flex items-center gap-2 px-2 py-1 rounded-btn ${
                        choice.code === locale ? 'bg-base-200 text-primary font-medium' : 'hover:bg-base-200'
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
        </nav>
      </header>
      {update.visible && (
        <div className="alert alert-info rounded-none sticky top-[64px] z-40">
          <div className="flex flex-1 flex-wrap items-center gap-3">
            <span>
              {t('app.update.available')}{' '}
              <span className="font-mono">{version?.version ?? t('app.update.current')}</span>
              {' → '}
              <span className="font-mono">{update.availableVersion}</span>
            </span>
            <div className="flex gap-2 ml-auto">
              <button className="btn btn-sm btn-primary" onClick={update.reload}>{t('app.update.refresh')}</button>
              <button className="btn btn-sm" onClick={update.dismiss}>{t('app.update.later')}</button>
            </div>
          </div>
        </div>
      )}
      <main className="px-4 py-6 pb-16">
        <Outlet />
      </main>
      <footer className="bt border-base-300 bg-base-100 text-sm text-base-content/70 w-full py-2 px-4 fixed bottom-0 left-0 flex items-center justify-between">
        <span>{t('app.footer.copyright')}</span>
        <div className="flex items-center gap-4">
          <a
            className="link flex items-center gap-1"
            href={repositoryUrl}
            target="_blank"
            rel="noreferrer"
            aria-label={t('app.footer.githubAria')}
          >
            <Icon icon="mdi:github" className="h-4 w-4" aria-hidden />
            <span>GitHub</span>
          </a>
          <span>
            {version && tagVersion ? (
              <a
                className="link font-mono"
                href={`${repositoryUrl}/releases/tag/${tagVersion}`}
                target="_blank"
                rel="noreferrer"
              >
                {t('app.footer.versionLabel', { version: tagVersion })}
              </a>
            ) : (
              t('app.footer.loadingVersion')
            )}
          </span>
        </div>
      </footer>
    </div>
  )
}
