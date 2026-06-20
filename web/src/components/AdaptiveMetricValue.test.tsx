/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { AdaptiveMetricValue } from './AdaptiveMetricValue'

vi.mock('./AnimatedDigits', () => ({
  AnimatedDigits: ({ value }: { value: number | string }) => (
    <span data-testid="animated-digits">{String(value)}</span>
  ),
}))

let host: HTMLDivElement | null = null
let root: Root | null = null
let metricContainerWidth = 320
let metricMeasureWidths = new Map<string, number>()

class MockResizeObserver {
  static instances = new Set<MockResizeObserver>()

  private readonly callback: ResizeObserverCallback
  private readonly observed = new Set<Element>()

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
    MockResizeObserver.instances.add(this)
  }

  observe(target: Element) {
    this.observed.add(target)
  }

  unobserve(target: Element) {
    this.observed.delete(target)
  }

  disconnect() {
    this.observed.clear()
    MockResizeObserver.instances.delete(this)
  }

  static notify(target: Element) {
    for (const instance of MockResizeObserver.instances) {
      if (!instance.observed.has(target)) continue
      instance.callback(
        [{ target, contentRect: target.getBoundingClientRect() } as ResizeObserverEntry],
        instance as unknown as ResizeObserver,
      )
    }
  }
}

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  Object.defineProperty(window, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })
  Object.defineProperty(globalThis, 'ResizeObserver', {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  })

  Object.defineProperty(HTMLElement.prototype, 'clientWidth', {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricContainer === 'true') {
        return metricContainerWidth
      }
      return 0
    },
  })

  Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
    configurable: true,
    get() {
      if ((this as HTMLElement).dataset.adaptiveMetricMeasure === 'true') {
        const key =
          (this as HTMLElement).dataset.adaptiveMetricMeasureIndex ??
          (this as HTMLElement).textContent ??
          '0'
        return metricMeasureWidths.get(key) ?? 0
      }
      return 0
    },
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  metricContainerWidth = 320
  metricMeasureWidths = new Map([
    ['0', 120],
    ['1', 88],
    ['2', 80],
    ['3', 72],
  ])
  MockResizeObserver.instances.clear()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function getMetric() {
  const metric = host?.querySelector('[data-testid="adaptive-metric"]')
  if (!(metric instanceof HTMLElement)) {
    throw new Error('Missing adaptive metric')
  }
  return metric
}

function getVisibleMetricText() {
  const visible = host?.querySelector('[data-adaptive-metric-visible="true"]')
  if (!(visible instanceof HTMLElement)) {
    throw new Error('Missing visible metric element')
  }
  return visible.textContent ?? ''
}

function getMeasure() {
  const measure = host?.querySelector('[data-adaptive-metric-measure="true"]')
  if (!(measure instanceof HTMLElement)) {
    throw new Error('Missing measure element')
  }
  return measure
}

describe('AdaptiveMetricValue', () => {
  it('switches to compact notation when the measured text widens without a container resize', () => {
    metricMeasureWidths = new Map([
      ['0', 120],
      ['1', 96],
      ['2', 88],
      ['3', 72],
    ])

    render(
      <AdaptiveMetricValue
        value={1314275579}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('false')

    metricMeasureWidths.set('0', 400)
    act(() => {
      MockResizeObserver.notify(getMeasure())
    })

    expect(getMetric().dataset.compact).toBe('true')
    expect(getVisibleMetricText()).toContain('1.31B')
    expect(getMetric().dataset.compactPrecision).toBe('2')
    expect(getMetric().getAttribute('title')).toBe('1,314,275,579')
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull()
  })

  it('re-evaluates overflow on window resize even when ResizeObserver is available', () => {
    metricMeasureWidths = new Map([
      ['0', 120],
      ['1', 96],
      ['2', 88],
      ['3', 72],
    ])

    render(
      <AdaptiveMetricValue
        value={1314275579}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('false')

    metricContainerWidth = 80
    act(() => {
      window.dispatchEvent(new Event('resize'))
    })

    expect(getMetric().dataset.compact).toBe('true')
  })

  it('keeps the short-scale compact suffix for zh overflow fallback', () => {
    metricContainerWidth = 100
    metricMeasureWidths = new Map([
      ['0', 400],
      ['1', 96],
      ['2', 88],
      ['3', 72],
    ])

    render(
      <AdaptiveMetricValue
        value={1314275579}
        localeTag="zh-CN"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('true')
    expect(getVisibleMetricText()).toContain('1.3B')
    expect(getVisibleMetricText()).toContain('B')
  })

  it('keeps AnimatedDigits only for non-compact number rendering', () => {
    metricMeasureWidths = new Map([
      ['0', 120],
      ['1', 96],
      ['2', 88],
      ['3', 72],
    ])

    render(
      <AdaptiveMetricValue
        value={12345}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(host?.querySelector('[data-testid="animated-digits"]')?.textContent).toBe('12,345')

    metricContainerWidth = 80
    metricMeasureWidths.set('0', 240)
    act(() => {
      MockResizeObserver.notify(getMeasure())
    })

    expect(getMetric().dataset.compact).toBe('true')
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull()
  })

  it('drops compact decimal precision to preserve the magnitude suffix when width is very tight', () => {
    metricContainerWidth = 76
    metricMeasureWidths = new Map([
      ['0', 220],
      ['1', 98],
      ['2', 86],
      ['3', 58],
    ])

    render(
      <AdaptiveMetricValue
        value={281_110_000}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('true')
    expect(getMetric().dataset.compactPrecision).toBe('0')
    expect(getVisibleMetricText()).toContain('281M')
    expect(getVisibleMetricText()).not.toContain('281.11M')
    expect(getMetric().getAttribute('title')).toBe('281,110,000')
  })
})
