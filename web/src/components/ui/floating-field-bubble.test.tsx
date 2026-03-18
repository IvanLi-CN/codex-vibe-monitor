/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { FloatingFieldBubble } from './floating-field-bubble'
import { FloatingFieldError } from './floating-field-error'

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
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  document.body.innerHTML = ''
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

describe('FloatingFieldBubble', () => {
  it('renders input-corner bubbles through a portal so overflow-hidden ancestors do not clip them', () => {
    render(
      <div className="relative overflow-hidden">
        <FloatingFieldBubble message="Body level warning" variant="warning" />
      </div>,
    )

    const bubble = document.body.querySelector('[role="status"]')

    expect(bubble).toBeInstanceOf(HTMLElement)
    expect(host?.querySelector('[role="status"]')).toBeNull()
    expect((bubble as HTMLElement | null)?.style.backgroundColor).toBe(
      'color-mix(in oklab, oklch(var(--color-warning)) 72%, oklch(var(--color-warning-content)))',
    )
    expect(bubble?.getAttribute('data-side')).not.toBeNull()
  })

  it('renders label-inline bubbles in flow with the requested variant', () => {
    render(
      <FloatingFieldBubble
        message="Looks good"
        variant="success"
        placement="label-inline"
      />,
    )

    const bubble = document.body.querySelector('[role="status"]')
    const arrow = document.body.querySelector('[data-bubble-arrow="true"]')

    expect(bubble).toBeInstanceOf(HTMLElement)
    expect(host?.querySelector('[role="status"]')).toBeNull()
    expect((bubble as HTMLElement | null)?.style.backgroundColor).toBe(
      'oklch(var(--color-success) / 0.40)',
    )
    expect(bubble?.getAttribute('data-side')).toBe('left')
    expect(arrow).toBeInstanceOf(SVGElement)
    expect((arrow as SVGElement | null)?.style.fill).toBe('oklch(var(--color-success) / 0.40)')
  })

  it('gives neutral bubbles a subtle tinted surface instead of a plain transparent panel', () => {
    render(
      <FloatingFieldBubble
        message="Heads up"
        variant="neutral"
        placement="label-inline"
      />,
    )

    const bubble = document.body.querySelector('[role="status"]')

    expect(bubble).toBeInstanceOf(HTMLElement)
    expect((bubble as HTMLElement | null)?.style.backgroundColor).toBe(
      'color-mix(in oklab, oklch(var(--color-base-100)) 88%, oklch(var(--color-primary)) 12%)',
    )
  })

  it('inherits the nearest theme scope so dark panels do not render with light-theme bubble tokens', () => {
    render(
      <div data-theme="vibe-dark">
        <FloatingFieldBubble
          message="Heads up"
          variant="warning"
          placement="label-inline"
        />
      </div>,
    )

    const bubble = document.body.querySelector('[role="status"]')
    const arrow = document.body.querySelector('[data-bubble-arrow="true"]')

    expect(bubble).toBeInstanceOf(HTMLElement)
    expect(bubble?.getAttribute('data-theme')).toBe('vibe-dark')
    expect(arrow?.getAttribute('data-theme')).toBe('vibe-dark')
    expect((bubble as HTMLElement | null)?.style.backgroundColor).toBe(
      'color-mix(in oklab, oklch(var(--color-base-200)) 62%, oklch(var(--color-warning)) 38%)',
    )
    expect((arrow as SVGElement | null)?.style.fill).toBe(
      'color-mix(in oklab, oklch(var(--color-base-200)) 62%, oklch(var(--color-warning)) 38%)',
    )
  })

  it('keeps FloatingFieldError as an error-variant compatibility wrapper', () => {
    render(
      <div className="relative">
        <FloatingFieldError message="Duplicate display name" />
      </div>,
    )

    const bubble = document.body.querySelector('[role="alert"]')

    expect(bubble).toBeInstanceOf(HTMLElement)
    expect((bubble as HTMLElement | null)?.style.backgroundColor).toBe(
      'oklch(var(--color-error) / 0.40)',
    )
    expect(bubble?.querySelector('[data-bubble-arrow="true"]')).toBeInstanceOf(SVGElement)
  })
})
