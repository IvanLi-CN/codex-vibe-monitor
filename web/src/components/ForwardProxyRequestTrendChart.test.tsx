/** @vitest-environment jsdom */
import { act } from 'react'
import type { ReactNode } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { ForwardProxyRequestTrendChart } from './ForwardProxyRequestTrendChart'

class MockPointerEvent extends MouseEvent {
  pointerType: string

  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init)
    this.pointerType = init.pointerType ?? 'mouse'
  }
}

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  Object.defineProperty(window, 'PointerEvent', {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  })
  Object.defineProperty(globalThis, 'PointerEvent', {
    configurable: true,
    writable: true,
    value: MockPointerEvent,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  root = null
  host = null
})

function render(ui: ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function mockRect(element: Element, rect: Partial<DOMRect> & { left: number; top: number; width: number; height: number }) {
  const fullRect = {
    left: rect.left,
    top: rect.top,
    width: rect.width,
    height: rect.height,
    right: rect.left + rect.width,
    bottom: rect.top + rect.height,
    x: rect.left,
    y: rect.top,
    toJSON: () => ({}),
  }
  Object.defineProperty(element, 'getBoundingClientRect', {
    configurable: true,
    value: () => fullRect,
  })
}

describe('ForwardProxyRequestTrendChart', () => {
  it('shows the shared inline tooltip details on hover for dialog charts', () => {
    render(
      <ForwardProxyRequestTrendChart
        buckets={[
          {
            bucketStart: '2026-03-01T00:00:00.000Z',
            bucketEnd: '2026-03-01T01:00:00.000Z',
            successCount: 12,
            failureCount: 1,
          },
          {
            bucketStart: '2026-03-01T01:00:00.000Z',
            bucketEnd: '2026-03-01T02:00:00.000Z',
            successCount: 9,
            failureCount: 0,
          },
        ]}
        scaleMax={13}
        localeTag="en-US"
        tooltipLabels={{
          success: 'Success',
          failure: 'Failure',
          total: 'Total requests',
        }}
        ariaLabel="JP Edge 01 Last 24h request volume chart"
        interactionHint="Hover or tap for details. Focus the chart and use arrow keys to switch points."
        variant="dialog"
        dataChartKind="proxy-binding-request-trend"
      />,
    )

    const container = document.querySelector(
      '[aria-label="JP Edge 01 Last 24h request volume chart"]',
    ) as HTMLElement | null
    const firstBar = container?.querySelector('[data-inline-chart-index="0"]') as HTMLElement | null

    expect(container).not.toBeNull()
    expect(firstBar).not.toBeNull()

    mockRect(container!, { left: 0, top: 0, width: 280, height: 96 })
    mockRect(firstBar!, { left: 24, top: 28, width: 10, height: 32 })

    act(() => {
      firstBar?.dispatchEvent(
        new MouseEvent('mouseover', {
          bubbles: true,
          clientX: 28,
          clientY: 36,
        }),
      )
    })

    const tooltip = document.body.querySelector('[role="tooltip"]') as HTMLElement | null
    expect(tooltip).not.toBeNull()
    expect(tooltip?.textContent).toContain('Success')
    expect(tooltip?.textContent).toContain('12')
    expect(tooltip?.textContent).toContain('Failure')
    expect(tooltip?.textContent).toContain('1')
    expect(tooltip?.textContent).toContain('Total requests')
    expect(tooltip?.textContent).toContain('13')
  })
})
