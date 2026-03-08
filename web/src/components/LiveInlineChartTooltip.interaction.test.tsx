/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { InlineChartTooltipSurface } from './ui/inline-chart-tooltip'
import { useInlineChartInteraction } from './ui/use-inline-chart-interaction'

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

function click(element: Element) {
  act(() => {
    element.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
}

function pointerDownOutside() {
  act(() => {
    document.body.dispatchEvent(new MockPointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
  })
}

function InteractionHarness({ itemCount, defaultIndex }: { itemCount: number; defaultIndex: number }) {
  const api = useInlineChartInteraction({ itemCount, defaultIndex })
  const containerProps = api.getContainerProps({ ariaLabel: 'Harness chart', describedBy: 'hint' })
  const itemProps = Array.from({ length: itemCount }, (_, index) => api.getItemProps(index))

  return (
    <div>
      <div data-testid="surface" ref={api.containerRef} {...containerProps}>
        {itemProps.map((props, index) => (
          <div key={index} data-testid={`item-${index}`} ref={props.ref} />
        ))}
      </div>
      <button data-testid="hover-0" onClick={() => itemProps[0]?.onMouseEnter({ clientX: 28, clientY: 40 } as never)} />
      <button data-testid="move-0" onClick={() => itemProps[0]?.onMouseMove({ clientX: 36, clientY: 44 } as never)} />
      <button data-testid="leave" onClick={() => containerProps.onMouseLeave()} />
      <button data-testid="focus" onClick={() => containerProps.onFocus()} />
      <button
        data-testid="key-left"
        onClick={() => containerProps.onKeyDown({ key: 'ArrowLeft', preventDefault() {} } as never)}
      />
      <button
        data-testid="key-escape"
        onClick={() => containerProps.onKeyDown({ key: 'Escape', preventDefault() {} } as never)}
      />
      <button
        data-testid="touch-1"
        onClick={() => {
          itemProps[1]?.onTouchStart()
          itemProps[1]?.onClick()
        }}
      />
      <output data-testid="state">{JSON.stringify({ ...api.state, anchor: api.anchor })}</output>
    </div>
  )
}

function state() {
  const node = document.querySelector('[data-testid="state"]')
  return node ? JSON.parse(node.textContent ?? '{}') : null
}

function TooltipHarness() {
  return (
    <InlineChartTooltipSurface
      items={[
        { title: 'Window A', rows: [{ label: 'Success', value: '2' }] },
        { title: 'Window B', rows: [{ label: 'Success', value: '4' }, { label: 'Failure', value: '1' }] },
      ]}
      defaultIndex={1}
      ariaLabel="Harness tooltip chart"
      interactionHint="Use arrow keys to switch"
    >
      {({ getItemProps }) => (
        <div data-testid="tooltip-surface" className="relative h-20 w-40">
          {Array.from({ length: 2 }, (_, index) => {
            const { ref, ...itemProps } = getItemProps(index)
            return <div key={index} data-testid={`tooltip-item-${index}`} ref={ref} {...itemProps} />
          })}
        </div>
      )}
    </InlineChartTooltipSurface>
  )
}

describe('Live inline chart tooltip interactions', () => {
  it('tracks hover open, move, and close for the request chart flow', () => {
    render(<InteractionHarness itemCount={2} defaultIndex={1} />)

    const surface = document.querySelector('[data-testid="surface"]') as HTMLElement
    const item0 = document.querySelector('[data-testid="item-0"]') as HTMLElement
    const item1 = document.querySelector('[data-testid="item-1"]') as HTMLElement
    mockRect(surface, { left: 0, top: 0, width: 260, height: 96 })
    mockRect(item0, { left: 24, top: 28, width: 8, height: 40 })
    mockRect(item1, { left: 40, top: 28, width: 8, height: 40 })

    click(document.querySelector('[data-testid="hover-0"]')!)
    expect(state()).toMatchObject({ activeIndex: 0, isOpen: true, isPinned: false, anchor: { x: 28, y: 40 } })

    click(document.querySelector('[data-testid="move-0"]')!)
    expect(state()).toMatchObject({ activeIndex: 0, isOpen: true, isPinned: false, anchor: { x: 36, y: 44 } })

    click(document.querySelector('[data-testid="leave"]')!)
    expect(state()).toMatchObject({ activeIndex: null, isOpen: false, isPinned: false, anchor: null })
  })

  it('uses focus and arrow keys to switch points on the weight chart flow', () => {
    render(<InteractionHarness itemCount={2} defaultIndex={1} />)

    const surface = document.querySelector('[data-testid="surface"]') as HTMLElement
    const item0 = document.querySelector('[data-testid="item-0"]') as HTMLElement
    const item1 = document.querySelector('[data-testid="item-1"]') as HTMLElement
    mockRect(surface, { left: 0, top: 0, width: 260, height: 96 })
    mockRect(item0, { left: 32, top: 24, width: 20, height: 40 })
    mockRect(item1, { left: 64, top: 24, width: 20, height: 40 })

    click(document.querySelector('[data-testid="focus"]')!)
    expect(state()).toMatchObject({ activeIndex: 1, isOpen: true, isPinned: false, anchor: { x: 74, y: 44 } })

    click(document.querySelector('[data-testid="key-left"]')!)
    expect(state()).toMatchObject({ activeIndex: 0, isOpen: true, isPinned: false, anchor: { x: 42, y: 44 } })

    click(document.querySelector('[data-testid="key-escape"]')!)
    expect(state()).toMatchObject({ activeIndex: null, isOpen: false, isPinned: false, anchor: null })
  })

  it('pins and dismisses the prompt-cache tap flow', () => {
    render(<InteractionHarness itemCount={2} defaultIndex={1} />)

    const surface = document.querySelector('[data-testid="surface"]') as HTMLElement
    const item0 = document.querySelector('[data-testid="item-0"]') as HTMLElement
    const item1 = document.querySelector('[data-testid="item-1"]') as HTMLElement
    mockRect(surface, { left: 0, top: 0, width: 260, height: 96 })
    mockRect(item0, { left: 20, top: 18, width: 90, height: 48 })
    mockRect(item1, { left: 118, top: 18, width: 96, height: 48 })

    click(document.querySelector('[data-testid="touch-1"]')!)
    expect(state()).toMatchObject({ activeIndex: 1, isOpen: true, isPinned: true, anchor: { x: 166, y: 42 } })

    pointerDownOutside()
    expect(state()).toMatchObject({ activeIndex: null, isOpen: false, isPinned: false, anchor: null })
  })

  it('exposes the active tooltip content to assistive technologies', () => {
    render(<TooltipHarness />)

    const surface = document.querySelector('[data-testid="tooltip-surface"]') as HTMLElement
    const container = document.querySelector('[aria-label="Harness tooltip chart"]') as HTMLElement
    const item0 = document.querySelector('[data-testid="tooltip-item-0"]') as HTMLElement
    const item1 = document.querySelector('[data-testid="tooltip-item-1"]') as HTMLElement
    mockRect(container, { left: 0, top: 0, width: 220, height: 96 })
    mockRect(surface, { left: 0, top: 0, width: 220, height: 96 })
    mockRect(item0, { left: 20, top: 24, width: 24, height: 40 })
    mockRect(item1, { left: 92, top: 24, width: 24, height: 40 })

    act(() => {
      container.focus()
    })

    const tooltip = document.querySelector('[role="tooltip"]') as HTMLElement | null
    const liveRegion = Array.from(document.querySelectorAll('.sr-only')).find((node) => node.textContent?.includes('Window B')) as HTMLElement | undefined
    const describedBy = container.getAttribute('aria-describedby') ?? ''

    expect(tooltip).not.toBeNull()
    expect(tooltip?.textContent).toContain('Window B')
    expect(tooltip?.getAttribute('aria-hidden')).toBe('false')
    expect(liveRegion?.getAttribute('aria-live')).toBe('polite')
    expect(liveRegion?.textContent).toContain('Failure 1')
    expect(describedBy).toContain(liveRegion?.id ?? '')
  })
})
