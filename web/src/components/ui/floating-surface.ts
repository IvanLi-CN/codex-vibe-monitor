import type { CSSProperties } from 'react'

export type FloatingSurfaceTone = 'neutral' | 'primary' | 'info' | 'success' | 'warning' | 'error'
export type FloatingSurfaceTheme = 'vibe-light' | 'vibe-dark' | undefined

const floatingSurfaceBackdropFilter = 'blur(18px) saturate(160%)'

function toneColorToken(tone: FloatingSurfaceTone) {
  switch (tone) {
    case 'primary':
      return '--color-primary'
    case 'info':
      return '--color-info'
    case 'success':
      return '--color-success'
    case 'warning':
      return '--color-warning'
    case 'error':
      return '--color-error'
    default:
      return '--color-primary'
  }
}

function backgroundMix(tone: FloatingSurfaceTone, theme: FloatingSurfaceTheme) {
  const isDark = theme === 'vibe-dark'
  const baseColor = isDark
    ? 'oklch(var(--color-base-200) / 0.94)'
    : 'oklch(var(--color-base-100) / 0.95)'
  const toneColor = `oklch(var(${toneColorToken(tone)}) / ${isDark ? '0.32' : '0.22'})`

  switch (tone) {
    case 'primary':
      return `color-mix(in oklab, ${baseColor} 78%, ${toneColor} 22%)`
    case 'warning':
      return `color-mix(in oklab, ${baseColor} 81%, ${toneColor} 19%)`
    case 'error':
      return `color-mix(in oklab, ${baseColor} 82%, ${toneColor} 18%)`
    case 'success':
      return `color-mix(in oklab, ${baseColor} 84%, ${toneColor} 16%)`
    case 'info':
      return `color-mix(in oklab, ${baseColor} 84%, ${toneColor} 16%)`
    default:
      return `color-mix(in oklab, ${baseColor} 86%, ${toneColor} 14%)`
  }
}

function borderMix(tone: FloatingSurfaceTone, theme: FloatingSurfaceTheme) {
  const isDark = theme === 'vibe-dark'
  const toneColor = `oklch(var(${toneColorToken(tone)}) / ${isDark ? '0.72' : '0.56'})`
  const baseColor = isDark
    ? 'oklch(var(--color-base-content) / 0.22)'
    : 'oklch(var(--color-base-content) / 0.14)'

  switch (tone) {
    case 'primary':
      return `color-mix(in oklab, ${toneColor} 64%, ${baseColor} 36%)`
    case 'warning':
      return `color-mix(in oklab, ${toneColor} 58%, ${baseColor} 42%)`
    case 'error':
      return `color-mix(in oklab, ${toneColor} 54%, ${baseColor} 46%)`
    case 'success':
      return `color-mix(in oklab, ${toneColor} 52%, ${baseColor} 48%)`
    case 'info':
      return `color-mix(in oklab, ${toneColor} 52%, ${baseColor} 48%)`
    default:
      return `color-mix(in oklab, ${toneColor} 46%, ${baseColor} 54%)`
  }
}

function insetRingMix(tone: FloatingSurfaceTone, theme: FloatingSurfaceTheme) {
  const isDark = theme === 'vibe-dark'
  const toneColor = `oklch(var(${toneColorToken(tone)}) / ${isDark ? '0.24' : '0.18'})`
  const baseColor = isDark ? 'rgba(255,255,255,0.08)' : 'rgba(255,255,255,0.34)'
  return `color-mix(in srgb, ${toneColor} 58%, ${baseColor} 42%)`
}

function floatingSurfaceShadow(tone: FloatingSurfaceTone, theme: FloatingSurfaceTheme) {
  const isDark = theme === 'vibe-dark'
  const ambient = isDark
    ? '0 24px 58px rgba(2, 6, 23, 0.48), 0 12px 26px rgba(2, 6, 23, 0.28)'
    : '0 22px 50px rgba(15, 23, 42, 0.16), 0 10px 24px rgba(15, 23, 42, 0.10)'
  const ring = `inset 0 0 0 1px ${insetRingMix(tone, theme)}`
  const highlight = isDark ? 'inset 0 1px 0 rgba(255,255,255,0.08)' : 'inset 0 1px 0 rgba(255,255,255,0.42)'
  return `${ambient}, ${ring}, ${highlight}`
}

export function floatingSurfaceStyle(
  tone: FloatingSurfaceTone = 'neutral',
  theme?: FloatingSurfaceTheme,
): CSSProperties {
  return {
    backgroundColor: backgroundMix(tone, theme),
    borderColor: borderMix(tone, theme),
    boxShadow: floatingSurfaceShadow(tone, theme),
    backdropFilter: floatingSurfaceBackdropFilter,
    WebkitBackdropFilter: floatingSurfaceBackdropFilter,
  }
}

export function floatingSurfaceArrowStyle(
  tone: FloatingSurfaceTone = 'neutral',
  theme?: FloatingSurfaceTheme,
): CSSProperties {
  return {
    fill: backgroundMix(tone, theme),
    stroke: borderMix(tone, theme),
    strokeWidth: '0.75px',
  }
}

