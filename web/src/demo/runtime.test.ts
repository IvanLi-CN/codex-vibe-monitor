import { afterEach, describe, expect, it, vi } from 'vitest'
import { appRuntime, sceneFromLocation, themeFromLocation } from './runtime'

afterEach(() => {
  vi.unstubAllEnvs()
})

describe('demo runtime selection', () => {
  it('allows only live and demo runtimes', () => {
    vi.stubEnv('VITE_APP_RUNTIME', 'demo')
    expect(appRuntime()).toBe('demo')

    vi.stubEnv('VITE_APP_RUNTIME', 'unsupported')
    expect(() => appRuntime()).toThrow('Unsupported VITE_APP_RUNTIME')
  })

  it('uses only hash query state for shareable scene and theme', () => {
    const location = new URL('https://demo.invalid/#/records?demoScene=network-failure&demoTheme=dark') as unknown as Location

    expect(sceneFromLocation(location)).toBe('network-failure')
    expect(themeFromLocation(location)).toBe('dark')
  })
})
