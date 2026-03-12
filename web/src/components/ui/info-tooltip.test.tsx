/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { InfoTooltip } from './info-tooltip'

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
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function createRect({ left, top, width, height }: { left: number; top: number; width: number; height: number }): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    width,
    height,
    right: left + width,
    bottom: top + height,
    toJSON: () => ({ left, top, width, height, right: left + width, bottom: top + height }),
  } as DOMRect
}

describe('InfoTooltip', () => {
  it('opens on click and closes when clicking outside while keeping tooltip semantics', () => {
    render(<InfoTooltip label="Explain notice" content="Current results stay on the latest searched snapshot." />)

    const button = host?.querySelector('button')
    const tooltip = host?.querySelector('[role="tooltip"]')

    expect(button).toBeInstanceOf(HTMLButtonElement)
    expect(tooltip).toBeInstanceOf(HTMLElement)
    expect(tooltip?.getAttribute('aria-hidden')).toBe('true')

    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')
    expect(button?.getAttribute('aria-describedby')).toBe(tooltip?.id)

    act(() => {
      document.body.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    })

    expect(tooltip?.getAttribute('aria-hidden')).toBe('true')
    expect(button?.getAttribute('aria-describedby')).toBeNull()
  })

  it('flips upward near the viewport bottom and keeps the icon size class', () => {
    const originalInnerWidth = window.innerWidth
    const originalInnerHeight = window.innerHeight
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 320 })
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 220 })

    render(<InfoTooltip label="Explain notice" content="Current results stay on the latest searched snapshot." />)

    const rootEl = host?.firstElementChild as HTMLElement | null
    const button = host?.querySelector('button')
    const tooltip = host?.querySelector('[role="tooltip"]') as HTMLElement | null

    expect(rootEl).toBeInstanceOf(HTMLElement)
    expect(button).toBeInstanceOf(HTMLButtonElement)
    expect(tooltip).toBeInstanceOf(HTMLElement)
    expect(host?.innerHTML).toContain('h-[18px] w-[18px]')

    Object.defineProperty(rootEl, 'getBoundingClientRect', {
      configurable: true,
      value: () => createRect({ left: 140, top: 184, width: 20, height: 20 }),
    })
    Object.defineProperty(tooltip, 'getBoundingClientRect', {
      configurable: true,
      value: () => createRect({ left: 24, top: 212, width: 256, height: 52 }),
    })

    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')
    expect(tooltip?.className).toContain('bottom-[calc(100%+0.45rem)]')
    expect(tooltip?.className).not.toContain('top-[calc(100%+0.45rem)]')

    Object.defineProperty(window, 'innerWidth', { configurable: true, value: originalInnerWidth })
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: originalInnerHeight })
  })
})
