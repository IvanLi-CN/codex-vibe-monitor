import { useLayoutEffect, useState } from 'react'
import type { BubbleTheme } from './bubble'

export function usePortaledTheme(anchor: HTMLElement | null) {
  const [theme, setTheme] = useState<BubbleTheme>(() => {
    if (typeof document === 'undefined') {
      return undefined
    }
    const rootTheme = document.documentElement.getAttribute('data-theme')
    return rootTheme === 'vibe-light' || rootTheme === 'vibe-dark' ? rootTheme : undefined
  })

  useLayoutEffect(() => {
    if (typeof document === 'undefined' || typeof MutationObserver === 'undefined') {
      return
    }

    const scopedThemeNode = anchor?.closest<HTMLElement>('[data-theme]') ?? document.documentElement

    const syncTheme = () => {
      const scopedTheme = scopedThemeNode.getAttribute('data-theme')
      setTheme(scopedTheme === 'vibe-light' || scopedTheme === 'vibe-dark' ? scopedTheme : undefined)
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
