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
    hoverHint: 'Hover or long-press to open the menu.',
  }
}

describe('AccountTagContextChip', () => {
  it('opens the context menu on hover', () => {
    render(
      <AccountTagContextChip
        name="vip-routing"
        labels={labels()}
        onRemove={() => undefined}
        onEdit={() => undefined}
      />,
    )

    const trigger = document.querySelector('button[aria-haspopup="menu"]') as HTMLElement
    act(() => {
      trigger.dispatchEvent(new MouseEvent('mouseover', { bubbles: true }))
    })

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

    const trigger = document.querySelector('button[aria-haspopup="menu"]') as HTMLElement
    act(() => {
      trigger.dispatchEvent(new MockPointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }))
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(460)
    })

    expect(document.querySelector('[role="menu"]')).not.toBeNull()
    expect(document.body.textContent).toContain('Hover or long-press to open the menu.')
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
