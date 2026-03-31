/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { ConcurrencyLimitSlider } from './ConcurrencyLimitSlider'

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

let root: Root | null = null
let host: HTMLDivElement | null = null

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  root = null
  host = null
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

describe('ConcurrencyLimitSlider', () => {
  it('renders the 30 label immediately before the unlimited column', () => {
    render(
      <ConcurrencyLimitSlider
        value={31}
        title="Concurrency limit"
        description="Use 1-30. The last step means unlimited."
        currentLabel="Current"
        unlimitedLabel="Unlimited"
        onChange={() => undefined}
      />,
    )

    const maxLabel = Array.from(document.querySelectorAll('span')).find(
      (element) => element.textContent?.trim() === '30',
    )

    expect(maxLabel).toBeTruthy()
    const legend = maxLabel?.parentElement ?? null
    const unlimitedLabel = Array.from(legend?.querySelectorAll('span') ?? []).find(
      (element) => element.textContent?.trim() === '∞',
    )

    expect(unlimitedLabel).toBeTruthy()
    expect(maxLabel?.style.gridColumn).toBe('30 / span 1')
    expect(unlimitedLabel?.style.gridColumn).toBe('31 / span 1')
    expect(unlimitedLabel?.getAttribute('aria-label')).toBe('Unlimited')
    expect(legend?.style.gridTemplateColumns).toBe(
      'repeat(30, minmax(0, 1fr)) max-content',
    )
  })
})
