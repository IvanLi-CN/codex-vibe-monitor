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
let metricMeasureWidth = 120

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
        return metricMeasureWidth
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
  metricMeasureWidth = 120
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

function getMeasure() {
  const measure = host?.querySelector('[data-adaptive-metric-measure="true"]')
  if (!(measure instanceof HTMLElement)) {
    throw new Error('Missing measure element')
  }
  return measure
}

describe('AdaptiveMetricValue', () => {
  it('switches to compact notation when the measured text widens without a container resize', () => {
    render(
      <AdaptiveMetricValue
        value={1314275579}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('false')

    metricMeasureWidth = 400
    act(() => {
      MockResizeObserver.notify(getMeasure())
    })

    expect(getMetric().dataset.compact).toBe('true')
    expect(getMetric().textContent).toContain('1.31B')
    expect(getMetric().getAttribute('title')).toBe('1,314,275,579')
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull()
  })

  it('re-evaluates overflow on window resize even when ResizeObserver is available', () => {
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
    metricMeasureWidth = 400

    render(
      <AdaptiveMetricValue
        value={1314275579}
        localeTag="zh-CN"
        data-testid="adaptive-metric"
      />,
    )

    expect(getMetric().dataset.compact).toBe('true')
    expect(getMetric().textContent).toContain('1.31B')
  })

  it('keeps AnimatedDigits only for non-compact number rendering', () => {
    render(
      <AdaptiveMetricValue
        value={12345}
        localeTag="en-US"
        data-testid="adaptive-metric"
      />,
    )

    expect(host?.querySelector('[data-testid="animated-digits"]')?.textContent).toBe('12,345')

    metricContainerWidth = 80
    metricMeasureWidth = 240
    act(() => {
      MockResizeObserver.notify(getMeasure())
    })

    expect(getMetric().dataset.compact).toBe('true')
    expect(host?.querySelector('[data-testid="animated-digits"]')).toBeNull()
  })
})
