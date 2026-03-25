/** @vitest-environment jsdom */
import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { I18nProvider } from '../../i18n'
import { floatingSurfaceStyle } from './floating-surface'
import { SystemNotificationProvider, useSystemNotifications } from './system-notifications'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  document.body.innerHTML = ''
  document.documentElement.removeAttribute('data-theme')
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function NotificationKickoff() {
  const { showMotherSwitchUndo } = useSystemNotifications()

  useEffect(() => {
    showMotherSwitchUndo({
      payload: {
        groupKey: 'tokyo-core',
        groupName: 'Tokyo Core',
        newMotherAccountId: 18,
        newMotherDisplayName: 'Codex Team - Tokyo',
        previousMotherAccountId: 11,
        previousMotherDisplayName: 'Codex Pro - Tokyo',
        hadNoMotherBefore: false,
      },
      onUndo: async () => undefined,
    })
  }, [showMotherSwitchUndo])

  return null
}

describe('System notifications', () => {
  it('renders warning toasts with the shared frosted surface tokens', () => {
    document.documentElement.setAttribute('data-theme', 'vibe-dark')

    render(
      <I18nProvider>
        <SystemNotificationProvider>
          <NotificationKickoff />
        </SystemNotificationProvider>
      </I18nProvider>,
    )

    const toast = document.body.querySelector('[role="status"]') as HTMLElement | null

    expect(toast).toBeInstanceOf(HTMLElement)
    expect(toast?.getAttribute('data-theme')).toBe('vibe-dark')
    expect(toast?.textContent).toContain('Tokyo Core')
    expect(toast?.style.backgroundColor).toBe(
      floatingSurfaceStyle('warning', 'vibe-dark').backgroundColor,
    )
    expect(toast?.style.backdropFilter).toBe(
      floatingSurfaceStyle('warning', 'vibe-dark').backdropFilter,
    )
    expect(toast?.style.borderColor).toBe(
      floatingSurfaceStyle('warning', 'vibe-dark').borderColor,
    )
  })
})
