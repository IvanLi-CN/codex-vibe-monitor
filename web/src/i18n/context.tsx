/* eslint-disable react-refresh/only-export-components */
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react'
import {
  FALLBACK_LOCALE,
  formatTranslation,
  supportedLocales,
  translations,
  type Locale,
  type TranslationKey,
  type TranslationValues,
} from './translations'

const STORAGE_KEY = 'codex-vibe-monitor.locale'
const DOC_LANG_MAP: Record<Locale, string> = {
  zh: 'zh-Hans',
  en: 'en',
}

interface I18nContextValue {
  locale: Locale
  setLocale: (next: Locale) => void
  t: (key: TranslationKey, values?: TranslationValues) => string
  availableLocales: readonly Locale[]
}

const I18nContext = createContext<I18nContextValue | undefined>(undefined)

function isLocale(value: unknown): value is Locale {
  return typeof value === 'string' && supportedLocales.includes(value as Locale)
}

function translate(locale: Locale, key: TranslationKey, values?: TranslationValues) {
  const dictionary = translations[locale] ?? translations[FALLBACK_LOCALE]
  const template = dictionary[key] ?? translations[FALLBACK_LOCALE][key] ?? key
  return formatTranslation(template, values)
}

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    if (typeof window !== 'undefined') {
      try {
        const cached = window.localStorage.getItem(STORAGE_KEY)
        if (isLocale(cached)) {
          return cached
        }
      } catch {
        // ignore storage failures (Safari private mode etc.)
      }
      const preferred = window.navigator.language || window.navigator.languages?.[0]
      if (preferred && preferred.toLowerCase().startsWith('zh')) {
        return 'zh'
      }
    }
    return supportedLocales[0]
  })

  const setLocale = useCallback((next: Locale) => {
    setLocaleState((current) => (current === next ? current : next))
    if (typeof window !== 'undefined') {
      try {
        window.localStorage.setItem(STORAGE_KEY, next)
      } catch {
        // suppress storage write errors
      }
    }
  }, [])

  useEffect(() => {
    if (typeof document === 'undefined') return
    const lang = DOC_LANG_MAP[locale] ?? DOC_LANG_MAP[FALLBACK_LOCALE]
    document.documentElement.lang = lang
    document.documentElement.setAttribute('data-locale', locale)
  }, [locale])

  const t = useCallback(
    (key: TranslationKey, values?: TranslationValues) => translate(locale, key, values),
    [locale],
  )

  const value = useMemo<I18nContextValue>(
    () => ({
      locale,
      setLocale,
      t,
      availableLocales: supportedLocales,
    }),
    [locale, setLocale, t],
  )

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useTranslation() {
  const context = useContext(I18nContext)
  if (!context) {
    throw new Error('useTranslation must be used within I18nProvider')
  }
  return context
}
