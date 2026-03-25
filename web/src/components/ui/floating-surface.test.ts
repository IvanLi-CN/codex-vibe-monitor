import { describe, expect, it } from 'vitest'
import {
  floatingSurfaceArrowStyle,
  floatingSurfaceStyle,
} from './floating-surface'

describe('floatingSurfaceStyle', () => {
  it('returns the shared blur stack for every tone', () => {
    expect(floatingSurfaceStyle('neutral').backdropFilter).toBe('blur(18px) saturate(160%)')
    expect(floatingSurfaceStyle('primary').WebkitBackdropFilter).toBe('blur(18px) saturate(160%)')
  })

  it('keeps light and dark surfaces distinct while preserving the same contract keys', () => {
    const light = floatingSurfaceStyle('neutral', 'vibe-light')
    const dark = floatingSurfaceStyle('neutral', 'vibe-dark')

    expect(light.backgroundColor).not.toBe(dark.backgroundColor)
    expect(light.borderColor).not.toBe(dark.borderColor)
    expect(light.boxShadow).not.toBe(dark.boxShadow)
  })

  it('lets tone accents change the surface tint without dropping the neutralized frosted base', () => {
    const neutral = floatingSurfaceStyle('neutral', 'vibe-light')
    const primary = floatingSurfaceStyle('primary', 'vibe-light')
    const warning = floatingSurfaceStyle('warning', 'vibe-light')

    expect(primary.backgroundColor).not.toBe(neutral.backgroundColor)
    expect(warning.borderColor).not.toBe(primary.borderColor)
    expect(primary.backgroundColor).toContain('color-mix')
    expect(warning.backgroundColor).toContain('color-mix')
  })
})

describe('floatingSurfaceArrowStyle', () => {
  it('mirrors the shared surface background and border for arrows', () => {
    const arrow = floatingSurfaceArrowStyle('warning', 'vibe-dark')
    const surface = floatingSurfaceStyle('warning', 'vibe-dark')

    expect(arrow.fill).toBe(surface.backgroundColor)
    expect(arrow.stroke).toBe(surface.borderColor)
    expect(arrow.strokeWidth).toBe('0.75px')
  })
})
