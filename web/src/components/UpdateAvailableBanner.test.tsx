/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { UpdateAvailableBanner } from './UpdateAvailableBanner'
import { floatingSurfaceStyle } from './ui/floating-surface'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  if (typeof MutationObserver === 'undefined') {
    const observerEntries: Array<{
      observer: MutationObserverMock
      target: Node
      callback: MutationCallback
    }> = []
    const originalSetAttribute = Element.prototype.setAttribute

    function notifyThemeMutation(target: Element, attributeName: string) {
      for (const entry of observerEntries) {
        if (entry.target !== target) {
          continue
        }
        entry.callback(
          [
            {
              type: 'attributes',
              attributeName,
              target,
              addedNodes: [] as unknown as NodeList,
              removedNodes: [] as unknown as NodeList,
              nextSibling: null,
              previousSibling: null,
              oldValue: null,
            } as MutationRecord,
          ],
          entry.observer,
        )
      }
    }

    class MutationObserverMock {
      constructor(private readonly callback: MutationCallback) {}

      observe(target: Node) {
        observerEntries.push({ observer: this, target, callback: this.callback })
      }

      disconnect() {
        for (let index = observerEntries.length - 1; index >= 0; index -= 1) {
          if (observerEntries[index]?.observer === this) {
            observerEntries.splice(index, 1)
          }
        }
      }

      takeRecords() {
        return []
      }
    }

    Object.defineProperty(Element.prototype, 'setAttribute', {
      configurable: true,
      writable: true,
      value(name: string, value: string) {
        originalSetAttribute.call(this, name, value)
        if (name === 'data-theme') {
          notifyThemeMutation(this, name)
        }
      },
    })

    Object.defineProperty(globalThis, 'MutationObserver', {
      configurable: true,
      writable: true,
      value: MutationObserverMock,
    })
  }
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  root = null
  host = null
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

describe('UpdateAvailableBanner', () => {
  it('renders update text, versions, and action labels', () => {
    render(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={vi.fn()}
        onDismiss={vi.fn()}
        labels={{
          available: '有新版本可用：',
          refresh: '立即刷新',
          later: '稍后',
        }}
      />,
    )

    const banner = host?.querySelector('[role="status"]')

    expect(banner?.textContent).toContain('有新版本可用：')
    expect(banner?.textContent).toContain('0.10.2')
    expect(banner?.textContent).toContain('0.10.4')
    expect(banner?.textContent).toContain('立即刷新')
    expect(banner?.textContent).toContain('稍后')
  })

  it('includes a11y status attributes', () => {
    render(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={vi.fn()}
        onDismiss={vi.fn()}
        labels={{
          available: 'A new version is available:',
          refresh: 'Refresh now',
          later: 'Later',
        }}
      />,
    )

    const banner = host?.querySelector('[role="status"]')

    expect(banner).toBeInstanceOf(HTMLElement)
    expect(banner?.getAttribute('aria-live')).toBe('polite')
  })

  it('uses the shared frosted primary surface instead of a low-alpha primary background', () => {
    render(
      <div data-theme="vibe-dark">
        <UpdateAvailableBanner
          currentVersion="0.10.2"
          availableVersion="0.10.4"
          onReload={vi.fn()}
          onDismiss={vi.fn()}
          labels={{
            available: 'A new version is available:',
            refresh: 'Refresh now',
            later: 'Later',
          }}
        />
      </div>,
    )

    const banner = host?.querySelector('[role="status"]') as HTMLElement | null

    expect(banner).toBeInstanceOf(HTMLElement)
    expect(banner?.style.backgroundColor).toBe(
      floatingSurfaceStyle('primary', 'vibe-dark').backgroundColor,
    )
    expect(banner?.style.backdropFilter).toBe(
      floatingSurfaceStyle('primary', 'vibe-dark').backdropFilter,
    )
    expect(banner?.style.borderColor).toBe(
      floatingSurfaceStyle('primary', 'vibe-dark').borderColor,
    )
  })

  it('tracks ancestor theme changes while the banner stays mounted', async () => {
    let themeWrapper: HTMLDivElement | null = null

    render(
      <div
        ref={(node) => {
          themeWrapper = node
        }}
        data-theme="vibe-light"
      >
        <UpdateAvailableBanner
          currentVersion="0.10.2"
          availableVersion="0.10.4"
          onReload={vi.fn()}
          onDismiss={vi.fn()}
          labels={{
            available: 'A new version is available:',
            refresh: 'Refresh now',
            later: 'Later',
          }}
        />
      </div>,
    )

    const banner = host?.querySelector('[role="status"]') as HTMLElement | null

    expect(banner?.style.backgroundColor).toBe(
      floatingSurfaceStyle('primary', 'vibe-light').backgroundColor,
    )

    await act(async () => {
      themeWrapper?.setAttribute('data-theme', 'vibe-dark')
      await Promise.resolve()
    })

    expect(banner?.style.backgroundColor).toBe(
      floatingSurfaceStyle('primary', 'vibe-dark').backgroundColor,
    )
  })

  it('binds refresh and dismiss buttons to provided callbacks', () => {
    const onReload = vi.fn()
    const onDismiss = vi.fn()

    render(
      <UpdateAvailableBanner
        currentVersion="0.10.2"
        availableVersion="0.10.4"
        onReload={onReload}
        onDismiss={onDismiss}
        labels={{
          available: '有新版本可用：',
          refresh: '立即刷新',
          later: '稍后',
        }}
      />,
    )

    const buttons = host?.querySelectorAll('button') ?? []
    const refreshButton = Array.from(buttons).find((button) => button.textContent?.trim() === '立即刷新')
    const laterButton = Array.from(buttons).find((button) => button.textContent?.trim() === '稍后')

    expect(refreshButton).toBeInstanceOf(HTMLButtonElement)
    expect(laterButton).toBeInstanceOf(HTMLButtonElement)

    act(() => {
      refreshButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      laterButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onReload).toHaveBeenCalledTimes(1)
    expect(onDismiss).toHaveBeenCalledTimes(1)
  })
})
