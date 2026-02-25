/* eslint-disable react-refresh/only-export-components */
import { createContext, useCallback, useContext, useLayoutEffect, useMemo, useState } from 'react'

export type ThemeMode = 'light' | 'dark'

interface ThemeContextValue {
  themeMode: ThemeMode
  setThemeMode: (next: ThemeMode) => void
  toggleTheme: () => void
}

const STORAGE_KEY = 'codex-vibe-monitor.theme-mode'
const FALLBACK_THEME: ThemeMode = 'light'

const DOC_THEME_ATTR: Record<ThemeMode, string> = {
  light: 'vibe-light',
  dark: 'vibe-dark',
}

const ThemeContext = createContext<ThemeContextValue | undefined>(undefined)

function isThemeMode(value: unknown): value is ThemeMode {
  return value === 'light' || value === 'dark'
}

function detectPreferredTheme(): ThemeMode {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return FALLBACK_THEME
  }
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

function applyTheme(next: ThemeMode) {
  if (typeof document === 'undefined') return
  document.documentElement.setAttribute('data-theme', DOC_THEME_ATTR[next])
  document.documentElement.setAttribute('data-color-mode', next)
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [themeMode, setThemeModeState] = useState<ThemeMode>(() => {
    if (typeof window !== 'undefined') {
      try {
        const cached = window.localStorage.getItem(STORAGE_KEY)
        if (isThemeMode(cached)) return cached
      } catch {
        // ignore storage read errors (private mode, quota limits)
      }
    }
    return detectPreferredTheme()
  })

  const setThemeMode = useCallback((next: ThemeMode) => {
    setThemeModeState((current) => {
      if (current === next) return current
      return next
    })

    applyTheme(next)

    if (typeof window !== 'undefined') {
      try {
        window.localStorage.setItem(STORAGE_KEY, next)
      } catch {
        // ignore storage write failures
      }
    }
  }, [])

  const toggleTheme = useCallback(() => {
    setThemeMode(themeMode === 'dark' ? 'light' : 'dark')
  }, [setThemeMode, themeMode])

  useLayoutEffect(() => {
    applyTheme(themeMode)
  }, [themeMode])

  const value = useMemo<ThemeContextValue>(
    () => ({
      themeMode,
      setThemeMode,
      toggleTheme,
    }),
    [themeMode, setThemeMode, toggleTheme],
  )

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
}

export function useTheme() {
  const context = useContext(ThemeContext)
  if (!context) {
    throw new Error('useTheme must be used within ThemeProvider')
  }
  return context
}
