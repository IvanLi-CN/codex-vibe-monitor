import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from '@iconify/react'
import { NavLink, Outlet } from 'react-router-dom'
import { subscribeToSse, requestImmediateReconnect } from '../lib/sse'
import useSseStatus from '../hooks/useSseStatus'
import useUpdateAvailable from '../hooks/useUpdateAvailable'
import { useProxyModelSettings } from '../hooks/useProxyModelSettings'
import { fetchVersion } from '../lib/api'
import type { VersionResponse } from '../lib/api'
import { frontendVersion, normalizeVersion } from '../lib/version'
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
const OFFLINE_NOTICE_THRESHOLD_MS = 2 * 60 * 1000

export function AppLayout() {
  const { t, locale, setLocale } = useTranslation()
  const [pulse, setPulse] = useState(false)
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const animationDurationMs = 1400
  const [versionInfo, setVersionInfo] = useState<VersionResponse | null>(null)
  const [backendLoading, setBackendLoading] = useState(true)
  const update = useUpdateAvailable()
  const {
    settings: proxySettings,
    isLoading: proxySettingsLoading,
    isSaving: proxySettingsSaving,
    error: proxySettingsError,
    update: updateProxySettings,
  } = useProxyModelSettings()
  const sseStatus = useSseStatus()
  const [proxySettingsOpen, setProxySettingsOpen] = useState(false)

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

  const handleToggleHijack = () => {
    if (!proxySettings) return
    void updateProxySettings({
      hijackEnabled: !proxySettings.hijackEnabled,
      mergeUpstreamEnabled: proxySettings.mergeUpstreamEnabled,
      enabledModels: proxySettings.enabledModels,
    })
  }

  const handleToggleMergeUpstream = () => {
    if (!proxySettings || !proxySettings.hijackEnabled) return
    void updateProxySettings({
      hijackEnabled: proxySettings.hijackEnabled,
      mergeUpstreamEnabled: !proxySettings.mergeUpstreamEnabled,
      enabledModels: proxySettings.enabledModels,
    })
  }

  const handleTogglePresetModel = (modelId: string) => {
    if (!proxySettings) return
    const enabled = new Set(proxySettings.enabledModels)
    if (enabled.has(modelId)) {
      enabled.delete(modelId)
    } else {
      enabled.add(modelId)
    }
    void updateProxySettings({
      hijackEnabled: proxySettings.hijackEnabled,
      mergeUpstreamEnabled: proxySettings.mergeUpstreamEnabled,
      enabledModels: proxySettings.models.filter((candidate) => enabled.has(candidate)),
    })
  }

  const enabledPresetModelSet = useMemo(
    () => new Set(proxySettings?.enabledModels ?? []),
    [proxySettings?.enabledModels],
  )

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
        <span aria-hidden>{' '}										{' 		'}→{' '}</span>
        <a
          className="link font-mono"
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
            <button
              type="button"
              className="btn btn-sm btn-ghost gap-2"
              aria-label={t('app.proxySettings.button')}
              onClick={() => setProxySettingsOpen(true)}
            >
              <Icon icon="mdi:tune-variant" className="h-5 w-5 text-base-content/70" aria-hidden />
              <span>{t('app.proxySettings.button')}</span>
            </button>
          </div>
        </nav>
      </header>
      <div className={`modal ${proxySettingsOpen ? 'modal-open' : ''}`}>
        <div className="modal-box max-w-xl">
          <h2 className="text-lg font-semibold">{t('app.proxySettings.title')}</h2>
          <p className="mt-1 text-sm text-base-content/70">{t('app.proxySettings.description')}</p>

          <div className="mt-4 space-y-4">
            {proxySettingsLoading && (
              <div className="text-sm text-base-content/70">{t('app.proxySettings.loading')}</div>
            )}

            {proxySettings && (
              <>
                <div className="rounded-box border border-base-300 p-3">
                  <div className="flex items-start justify-between gap-4">
                    <div>
                      <div className="font-medium">{t('app.proxySettings.hijackLabel')}</div>
                      <div className="text-sm text-base-content/70">{t('app.proxySettings.hijackHint')}</div>
                    </div>
                    <input
                      type="checkbox"
                      className="toggle toggle-primary mt-1"
                      checked={proxySettings.hijackEnabled}
                      disabled={proxySettingsSaving}
                      onChange={handleToggleHijack}
                    />
                  </div>
                </div>

                <div className="rounded-box border border-base-300 p-3">
                  <div className="flex items-start justify-between gap-4">
                    <div>
                      <div className="font-medium">{t('app.proxySettings.mergeLabel')}</div>
                      <div className="text-sm text-base-content/70">{t('app.proxySettings.mergeHint')}</div>
                      {!proxySettings.hijackEnabled && (
                        <div className="mt-1 text-xs text-warning">{t('app.proxySettings.mergeDisabledHint')}</div>
                      )}
                    </div>
                    <input
                      type="checkbox"
                      className="toggle toggle-primary mt-1"
                      checked={proxySettings.mergeUpstreamEnabled}
                      disabled={proxySettingsSaving || !proxySettings.hijackEnabled}
                      onChange={handleToggleMergeUpstream}
                    />
                  </div>
                </div>

                <div className="rounded-box border border-base-300 p-3">
                  <div className="mb-2 flex items-center justify-between gap-2">
                    <div className="font-medium">{t('app.proxySettings.presetModels')}</div>
                    <div className="text-xs text-base-content/60">
                      {t('app.proxySettings.enabledCount', {
                        count: proxySettings.enabledModels.length,
                        total: proxySettings.models.length,
                      })}
                    </div>
                  </div>
                  <div className="space-y-2">
                    {proxySettings.models.map((modelId) => {
                      const enabled = enabledPresetModelSet.has(modelId)
                      return (
                        <label
                          key={modelId}
                          className={`flex items-center justify-between gap-3 rounded-lg border px-3 py-2.5 transition-colors ${
                            enabled
                              ? 'border-primary/45 bg-primary/5'
                              : 'border-base-300 bg-base-100 hover:border-primary/35 hover:bg-base-200/50'
                          } ${proxySettingsSaving ? 'cursor-not-allowed opacity-65' : 'cursor-pointer'}`}
                        >
                          <div className="min-w-0 flex-1 truncate font-mono text-sm leading-5">{modelId}</div>
                          <span
                            className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium ${
                              enabled
                                ? 'border-success/45 bg-success/15 text-success'
                                : 'border-base-300 bg-base-200/70 text-base-content/60'
                            }`}
                          >
                            <Icon
                              icon={enabled ? 'mdi:check-circle' : 'mdi:close-circle-outline'}
                              className="h-3.5 w-3.5"
                            />
                            {enabled
                              ? t('app.proxySettings.modelEnabledBadge')
                              : t('app.proxySettings.modelDisabledBadge')}
                          </span>
                          <input
                            type="checkbox"
                            className="sr-only"
                            checked={enabled}
                            disabled={proxySettingsSaving}
                            onChange={() => handleTogglePresetModel(modelId)}
                            aria-label={`${modelId} ${
                              enabled
                                ? t('app.proxySettings.modelEnabledBadge')
                                : t('app.proxySettings.modelDisabledBadge')
                            }`}
                          />
                        </label>
                      )
                    })}
                  </div>
                  {proxySettings.enabledModels.length === 0 && (
                    <div className="mt-2 text-xs text-warning">{t('app.proxySettings.noneEnabledHint')}</div>
                  )}
                </div>
              </>
            )}

            {proxySettingsSaving && (
              <div className="text-sm text-base-content/70">{t('app.proxySettings.saving')}</div>
            )}

            {proxySettingsError && (
              <div className="text-sm text-error">
                {t(
                  proxySettings ? 'app.proxySettings.saveError' : 'app.proxySettings.loadError',
                  { error: proxySettingsError },
                )}
              </div>
            )}
          </div>

          <div className="modal-action">
            <button type="button" className="btn" onClick={() => setProxySettingsOpen(false)}>
              {t('app.proxySettings.close')}
            </button>
          </div>
        </div>
        <button
          type="button"
          className="modal-backdrop"
          aria-label={t('app.proxySettings.close')}
          onClick={() => setProxySettingsOpen(false)}
        />
      </div>
      {showOfflineBanner && (
        <div className="fixed top-[72px] left-1/2 z-[60] w-full max-w-3xl -translate-x-1/2 px-4">
          <div
            className="alert w-full flex flex-col gap-3 bg-warning/90 text-warning-content shadow-lg border border-warning/60 sm:flex-row sm:items-center"
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
            <button
              type="button"
              className="btn btn-sm btn-primary w-full sm:w-auto sm:ml-auto"
              onClick={handleManualReconnect}
            >
              {t('app.sse.banner.reconnectButton')}
            </button>
          </div>
        </div>
      )}
      {update.visible && (
        <div className="alert alert-info rounded-none sticky top-[64px] z-40">
          <div className="flex flex-1 flex-wrap items-center gap-3">
            <span>
              {t('app.update.available')}{' '}
              <span className="font-mono">{versionInfo?.backend ?? t('app.update.current')}</span>
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
          <div className="flex items-center gap-2">
            {sameVersion && normalizedBackendVersion ? (
              releaseLink ? (
                <a
                  className="link font-mono"
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
      </footer>
    </div>
  )
}
