/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { InfoTooltip } from './info-tooltip'

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  if (!('ResizeObserver' in globalThis)) {
    Object.defineProperty(globalThis, 'ResizeObserver', {
      configurable: true,
      writable: true,
      value: class ResizeObserver {
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    })
  }
})

afterEach(() => {
  vi.useRealTimers()
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

    expect(button).toBeInstanceOf(HTMLButtonElement)

    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const tooltip = document.body.querySelector('[role="tooltip"]')

    expect(tooltip).toBeInstanceOf(HTMLElement)
    expect(host?.contains(tooltip as Node)).toBe(false)
    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')
    expect(button?.getAttribute('aria-describedby')).toBe(tooltip?.id)

    act(() => {
      document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    })

    const closedTooltip = document.body.querySelector('[role="tooltip"]')

    expect(closedTooltip?.getAttribute('aria-hidden') ?? 'true').toBe('true')
    expect(button?.getAttribute('aria-describedby')).toBeNull()
  })

  it('opens into a portaled anchored bubble and keeps the icon size class', () => {
    const originalInnerWidth = window.innerWidth
    const originalInnerHeight = window.innerHeight
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 320 })
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 220 })

    render(<InfoTooltip label="Explain notice" content="Current results stay on the latest searched snapshot." />)

    const button = host?.querySelector('button')

    expect(button).toBeInstanceOf(HTMLButtonElement)
    expect(host?.innerHTML).toContain('h-[18px] w-[18px]')

    Object.defineProperty(button, 'getBoundingClientRect', {
      configurable: true,
      value: () => createRect({ left: 140, top: 184, width: 20, height: 20 }),
    })

    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const tooltip = document.body.querySelector('[role="tooltip"]') as HTMLElement | null

    expect(tooltip).toBeInstanceOf(HTMLElement)
    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')
    expect(tooltip?.getAttribute('data-side')).not.toBeNull()
    expect(host?.contains(tooltip as Node)).toBe(false)

    Object.defineProperty(window, 'innerWidth', { configurable: true, value: originalInnerWidth })
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: originalInnerHeight })
  })

  it('passes the nearest theme scope into the portaled tooltip surface', () => {
    render(
      <div data-theme="vibe-dark">
        <InfoTooltip label="Explain notice" content="Current results stay on the latest searched snapshot." />
      </div>,
    )

    const button = host?.querySelector('button')

    expect(button).toBeInstanceOf(HTMLButtonElement)

    act(() => {
      button?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const tooltip = document.body.querySelector('[role="tooltip"]')
    const arrow = document.body.querySelector('[data-bubble-arrow="true"]')

    expect(tooltip).toBeInstanceOf(HTMLElement)
    expect(tooltip?.getAttribute('data-theme')).toBe('vibe-dark')
    expect(arrow?.getAttribute('data-theme')).toBe('vibe-dark')
    expect((tooltip as HTMLElement | null)?.style.backgroundColor).toBe(
      'color-mix(in oklab, oklch(var(--color-base-200)) 86%, oklch(var(--color-primary)) 14%)',
    )
    expect((arrow as SVGElement | null)?.style.fill).toBe(
      'color-mix(in oklab, oklch(var(--color-base-200)) 86%, oklch(var(--color-primary)) 14%)',
    )
  })

  it('keeps hover tooltips open while moving the pointer into the bubble content', () => {
    vi.useFakeTimers()

    render(<InfoTooltip label="Explain notice" content="Current results stay on the latest searched snapshot." />)

    const button = host?.querySelector('button')
    const trigger = button?.closest('span')

    expect(button).toBeInstanceOf(HTMLButtonElement)
    expect(trigger).toBeInstanceOf(HTMLSpanElement)

    act(() => {
      button?.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }))
    })

    const tooltip = document.body.querySelector('[role="tooltip"]') as HTMLElement | null

    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')

    act(() => {
      button?.dispatchEvent(new MouseEvent('mouseout', { bubbles: true }))
      tooltip?.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }))
      vi.advanceTimersByTime(150)
    })

    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')

    act(() => {
      tooltip?.dispatchEvent(new MouseEvent('mouseout', { bubbles: true }))
      vi.advanceTimersByTime(150)
    })

    expect(tooltip?.getAttribute('aria-hidden')).toBe('true')

    vi.useRealTimers()
  })
})
