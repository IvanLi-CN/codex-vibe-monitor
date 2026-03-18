import { useLayoutEffect, useState } from 'react'

export function usePortaledTheme(anchor: HTMLElement | null) {
  const [theme, setTheme] = useState<string | undefined>(() => {
    if (typeof document === 'undefined') {
      return undefined
    }
    return document.documentElement.getAttribute('data-theme') ?? undefined
  })

  useLayoutEffect(() => {
    if (typeof document === 'undefined' || typeof MutationObserver === 'undefined') {
      return
    }

    const scopedThemeNode = anchor?.closest<HTMLElement>('[data-theme]') ?? document.documentElement

    const syncTheme = () => {
      setTheme(scopedThemeNode.getAttribute('data-theme') ?? undefined)
    }

    syncTheme()

    const observer = new MutationObserver(syncTheme)
    observer.observe(scopedThemeNode, {
      attributes: true,
      attributeFilter: ['data-theme'],
    })

    return () => observer.disconnect()
  }, [anchor])

  return theme
}
