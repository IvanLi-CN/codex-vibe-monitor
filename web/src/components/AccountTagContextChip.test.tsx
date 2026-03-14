/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { AccountTagContextChip } from './AccountTagContextChip'

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
  vi.useRealTimers()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function labels() {
  return {
    selectedFromCurrentPage: 'New',
    remove: 'Unlink tag',
    deleteAndRemove: 'Delete and unlink',
    edit: 'Edit routing rule',
  }
}

describe('AccountTagContextChip', () => {
  it('reveals the action button on hover and opens the menu on click', () => {
    render(
      <AccountTagContextChip
        name="vip-routing"
        labels={labels()}
        onRemove={() => undefined}
        onEdit={() => undefined}
      />,
    )

    const wrapper = document.querySelector('.relative.inline-flex') as HTMLElement
    const actionButton = document.querySelector('button[aria-haspopup="menu"]') as HTMLElement

    expect(actionButton.className).toContain('opacity-0')
    expect(actionButton.getAttribute('aria-expanded')).toBe('false')

    act(() => {
      wrapper.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: null }))
    })

    expect(actionButton.className).toContain('opacity-100')

    act(() => {
      actionButton.click()
    })

    expect(actionButton.getAttribute('aria-expanded')).toBe('true')
    expect(document.body.textContent).toContain('Unlink tag')
    expect(document.body.textContent).toContain('Edit routing rule')
  })

  it('opens the context menu after a touch long press', async () => {
    vi.useFakeTimers()
    render(
      <AccountTagContextChip
        name="vip-routing"
        labels={labels()}
        onRemove={() => undefined}
        onEdit={() => undefined}
      />,
    )

    const touchSurface = document.querySelector('.relative.inline-flex > .inline-flex') as HTMLElement
    act(() => {
      touchSurface.dispatchEvent(new MockPointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(460)
    })

    expect(document.querySelector('[role="menu"]')).not.toBeNull()
    expect(document.body.textContent).toContain('Unlink tag')
    expect(document.body.textContent).toContain('Edit routing rule')
  })

  it('uses the delete copy for tags created on the current page', () => {
    render(
      <AccountTagContextChip
        name="vip-routing"
        currentPageCreated
        defaultOpen
        labels={labels()}
        onRemove={() => undefined}
        onEdit={() => undefined}
      />,
    )

    expect(document.body.textContent).toContain('Delete and unlink')
    expect(document.body.textContent).toContain('New')
  })
})
