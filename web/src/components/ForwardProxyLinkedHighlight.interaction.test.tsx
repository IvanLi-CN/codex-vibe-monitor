/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { I18nProvider } from '../i18n'
import type { ForwardProxyLiveStatsResponse } from '../lib/api'
import { ForwardProxyLiveTable } from './ForwardProxyLiveTable'

let host: HTMLDivElement | null = null
let root: Root | null = null

class MockPointerEvent extends MouseEvent {
  pointerType: string

  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init)
    this.pointerType = init.pointerType ?? 'mouse'
  }
}

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

function render(ui: React.ReactNode) {
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

function hover(element: Element, clientX: number, clientY: number) {
  act(() => {
    element.dispatchEvent(new MockPointerEvent('pointerover', { bubbles: true, clientX, clientY, pointerType: 'mouse' }))
    element.dispatchEvent(new MockPointerEvent('pointerenter', { bubbles: true, clientX, clientY, pointerType: 'mouse' }))
    element.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, clientX, clientY }))
    element.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true, clientX, clientY }))
    element.dispatchEvent(new MockPointerEvent('pointermove', { bubbles: true, clientX, clientY, pointerType: 'mouse' }))
    element.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX, clientY }))
  })
}


function leave(element: Element) {
  act(() => {
    element.dispatchEvent(new MockPointerEvent('pointerout', { bubbles: true, pointerType: 'mouse' }))
    element.dispatchEvent(new MockPointerEvent('pointerleave', { bubbles: true, pointerType: 'mouse' }))
    element.dispatchEvent(new MouseEvent('mouseout', { bubbles: true }))
    element.dispatchEvent(new MouseEvent('mouseleave', { bubbles: true }))
  })
}

function keydown(element: Element, key: string) {
  act(() => {
    element.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key }))
  })
}

const stats: ForwardProxyLiveStatsResponse = {
  rangeStart: '2026-03-01T00:00:00Z',
  rangeEnd: '2026-03-02T00:00:00Z',
  bucketSeconds: 3600,
  nodes: [
    {
      key: 'proxy-a',
      source: 'manual',
      displayName: 'Proxy A',
      weight: 0.8,
      penalized: false,
      stats: {
        oneMinute: { attempts: 1, successRate: 0.86, avgLatencyMs: 163 },
        fifteenMinutes: { attempts: 1, successRate: 0.9, avgLatencyMs: 182 },
        oneHour: { attempts: 1, successRate: 0.91, avgLatencyMs: 195 },
        oneDay: { attempts: 1, successRate: 0.89, avgLatencyMs: 224 },
        sevenDays: { attempts: 1, successRate: 0.9, avgLatencyMs: 241 },
      },
      last24h: [
        { bucketStart: '2026-03-01T00:00:00Z', bucketEnd: '2026-03-01T01:00:00Z', successCount: 4, failureCount: 1 },
        { bucketStart: '2026-03-01T01:00:00Z', bucketEnd: '2026-03-01T02:00:00Z', successCount: 5, failureCount: 0 },
        { bucketStart: '2026-03-01T02:00:00Z', bucketEnd: '2026-03-01T03:00:00Z', successCount: 3, failureCount: 2 },
      ],
      weight24h: [
        {
          bucketStart: '2026-03-01T00:00:00Z',
          bucketEnd: '2026-03-01T01:00:00Z',
          sampleCount: 1,
          minWeight: 0.3,
          maxWeight: 0.3,
          avgWeight: 0.3,
          lastWeight: 0.3,
        },
        {
          bucketStart: '2026-03-01T01:00:00Z',
          bucketEnd: '2026-03-01T02:00:00Z',
          sampleCount: 1,
          minWeight: 0.8,
          maxWeight: 0.8,
          avgWeight: 0.8,
          lastWeight: 0.8,
        },
        {
          bucketStart: '2026-03-01T02:00:00Z',
          bucketEnd: '2026-03-01T03:00:00Z',
          sampleCount: 1,
          minWeight: 0.5,
          maxWeight: 0.5,
          avgWeight: 0.5,
          lastWeight: 0.5,
        },
      ],
    },
  ],
}

describe('ForwardProxyLiveTable linked chart highlight', () => {
  it('links request bars and weight trend points for the same row bucket', () => {
    render(
      <I18nProvider>
        <ForwardProxyLiveTable stats={stats} isLoading={false} error={null} />
      </I18nProvider>,
    )

    const requestSurface = document.querySelector('[aria-label="Proxy A 近 24 小时请求量图"]') as HTMLElement
    const weightSurface = document.querySelector('[aria-label="Proxy A 近 24 小时权重趋势图"]') as HTMLElement
    const requestChart = document.querySelector('[data-chart-kind="proxy-request-trend"]') as HTMLElement
    const weightChart = document.querySelector('[data-chart-kind="proxy-weight-trend"]') as SVGElement
    let requestBars = requestChart.querySelectorAll('[data-inline-chart-index]')
    const weightHits = weightChart.querySelectorAll('[data-inline-chart-index]')

    mockRect(requestSurface, { left: 640, top: 120, width: 220, height: 48 })
    mockRect(weightSurface, { left: 920, top: 120, width: 220, height: 48 })
    mockRect(requestBars[0]!, { left: 652, top: 126, width: 8, height: 40 })
    mockRect(requestBars[1]!, { left: 668, top: 126, width: 8, height: 40 })
    mockRect(requestBars[2]!, { left: 684, top: 126, width: 8, height: 40 })
    mockRect(weightHits[0]!, { left: 934, top: 126, width: 32, height: 40 })
    mockRect(weightHits[1]!, { left: 966, top: 126, width: 32, height: 40 })
    mockRect(weightHits[2]!, { left: 998, top: 126, width: 32, height: 40 })

    hover(requestBars[1]!, 672, 144)
    expect(weightChart.querySelector('line[stroke-dasharray="3 2"]')).not.toBeNull()

    leave(requestSurface)

    act(() => {
      weightSurface.focus()
    })
    keydown(weightSurface, 'Home')

    requestBars = requestChart.querySelectorAll('[data-inline-chart-index]')
    expect(requestBars[0]?.className).toContain('border-primary/60')
    expect(requestBars[1]?.className).not.toContain('border-primary/60')
  })
})
