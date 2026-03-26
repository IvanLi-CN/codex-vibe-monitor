/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { MultiSelectFilterCombobox, type MultiSelectFilterOption } from './MultiSelectFilterCombobox'

class MockPointerEvent extends MouseEvent {
  pointerType: string

  constructor(type: string, init: MouseEventInit & { pointerType?: string } = {}) {
    super(type, init)
    this.pointerType = init.pointerType ?? 'mouse'
  }
}

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
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
  Object.defineProperty(HTMLElement.prototype, 'scrollIntoView', {
    configurable: true,
    writable: true,
    value: () => undefined,
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

const options: MultiSelectFilterOption[] = [
  { value: 'working', label: 'Working' },
  { value: 'idle', label: 'Idle' },
  { value: 'rate_limited', label: 'Rate limited' },
]

function createHarness() {
  const onValueChangeSpy = vi.fn()

  function Harness() {
    const [value, setValue] = React.useState<string[]>([])
    return (
      <MultiSelectFilterCombobox
        options={options}
        value={value}
        placeholder="All work statuses"
        searchPlaceholder="Search work statuses"
        emptyLabel="No work statuses"
        clearLabel="Clear work status filters"
        ariaLabel="Work status"
        onValueChange={(nextValue) => {
          onValueChangeSpy(nextValue)
          setValue(nextValue)
        }}
      />
    )
  }

  return { Harness, onValueChangeSpy }
}

describe('MultiSelectFilterCombobox', () => {
  it('shows the first two selections in the trigger and clears them from the command action', () => {
    const { Harness, onValueChangeSpy } = createHarness()
    render(<Harness />)

    const trigger = document.querySelector('button[role="combobox"]') as HTMLButtonElement
    act(() => {
      trigger.click()
    })

    const clickItem = (matcher: RegExp) => {
      const item = Array.from(document.querySelectorAll('[cmdk-item]')).find((candidate) =>
        matcher.test(candidate.textContent || ''),
      )
      expect(item).toBeInstanceOf(HTMLElement)
      act(() => {
        item?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      })
    }

    clickItem(/^Rate limited$/i)
    clickItem(/^Working$/i)
    clickItem(/^Idle$/i)

    expect(onValueChangeSpy).toHaveBeenLastCalledWith(['rate_limited', 'working', 'idle'])
    expect(trigger.textContent).toContain('Working, Idle +1')

    const clearItem = Array.from(document.querySelectorAll('[cmdk-item]')).find((item) =>
      /Clear work status filters/i.test(item.textContent || ''),
    )
    expect(clearItem).toBeInstanceOf(HTMLElement)

    act(() => {
      clearItem?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onValueChangeSpy).toHaveBeenLastCalledWith([])
    expect(trigger.textContent).toContain('All work statuses')
  })
})
