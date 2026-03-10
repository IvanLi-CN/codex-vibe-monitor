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
})
