/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { AccountTagFilterCombobox } from './AccountTagFilterCombobox'
import type { TagSummary } from '../lib/api'

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

const tags: TagSummary[] = [
  {
    id: 1,
    name: 'vip',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 2,
    groupCount: 1,
    updatedAt: '2026-03-22T09:00:00.000Z',
  },
  {
    id: 2,
    name: 'burst-safe',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 3,
    groupCount: 1,
    updatedAt: '2026-03-22T09:00:00.000Z',
  },
  {
    id: 3,
    name: 'prod-apac',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 1,
    groupCount: 1,
    updatedAt: '2026-03-22T09:00:00.000Z',
  },
  {
    id: 4,
    name: 'sticky-pool',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 1,
    groupCount: 1,
    updatedAt: '2026-03-22T09:00:00.000Z',
  },
]

function createHarness() {
  const onValueChangeSpy = vi.fn()

  function Harness() {
    const [value, setValue] = React.useState<number[]>([])
    return (
      <AccountTagFilterCombobox
        tags={tags}
        value={value}
        prioritizedTagIds={[1, 2]}
        disabledTagIds={[3, 4]}
        placeholder="Choose tags"
        searchPlaceholder="Search tags"
        emptyLabel="No tags"
        clearLabel="Clear"
        ariaLabel="Tags"
        onValueChange={(nextValue) => {
          onValueChangeSpy(nextValue)
          setValue(nextValue)
        }}
      />
    )
  }

  return { Harness, onValueChangeSpy }
}

describe('AccountTagFilterCombobox', () => {
  it('shows prioritized removable tags first and disables the rest at the end', () => {
    const { Harness, onValueChangeSpy } = createHarness()
    render(<Harness />)

    const trigger = document.querySelector('button[role="combobox"]') as HTMLButtonElement
    act(() => {
      trigger.click()
    })

    const options = Array.from(document.querySelectorAll('[cmdk-item]')) as HTMLElement[]
    expect(options.map((option) => option.textContent?.trim())).toEqual([
      'burst-safe',
      'vip',
      'prod-apac',
      'sticky-pool',
    ])
    expect(options[0]?.getAttribute('aria-disabled')).toBe('false')
    expect(options[1]?.getAttribute('aria-disabled')).toBe('false')
    expect(options[2]?.getAttribute('aria-disabled')).toBe('true')
    expect(options[3]?.getAttribute('aria-disabled')).toBe('true')

    act(() => {
      options[2]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expect(onValueChangeSpy).not.toHaveBeenCalled()

    act(() => {
      options[0]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expect(onValueChangeSpy).toHaveBeenLastCalledWith([2])
    expect(trigger.textContent).toContain('burst-safe')
  })
})
