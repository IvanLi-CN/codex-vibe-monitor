/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { AccountTagField } from './AccountTagField'
import type { CreateTagPayload, TagDetail, TagSummary, UpdateTagPayload } from '../lib/api'

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

const labels = {
  label: 'Tags',
  add: 'Add tag',
  empty: 'No tag selected yet.',
  searchPlaceholder: 'Search existing tags...',
  searchEmpty: 'No matching tags.',
  createInline: (value: string) => (value ? `Create "${value}"` : 'Create new tag'),
  selectedFromCurrentPage: 'New',
  remove: 'Unlink tag',
  deleteAndRemove: 'Delete and unlink',
  edit: 'Edit routing rule',
  createTitle: 'Create tag',
  editTitle: 'Edit tag',
  dialogDescription: 'Configure the routing policy bound to this tag.',
  name: 'Name',
  namePlaceholder: 'vip-routing',
  guardEnabled: 'Conversation guard',
  lookbackHours: 'Lookback hours',
  maxConversations: 'Max conversations',
  allowCutOut: 'Allow cut out',
  allowCutIn: 'Allow cut in',
  cancel: 'Cancel',
  save: 'Save',
  createAction: 'Create',
  validation: 'Use positive integers for the guard values.',
}

const tags: TagSummary[] = [
  {
    id: 1,
    name: 'vip-routing',
    routingRule: {
      guardEnabled: false,
      lookbackHours: null,
      maxConversations: null,
      allowCutOut: true,
      allowCutIn: true,
    },
    accountCount: 3,
    groupCount: 1,
    updatedAt: '2026-03-14T15:20:00.000Z',
  },
  {
    id: 2,
    name: 'handoff-blocked',
    routingRule: {
      guardEnabled: true,
      lookbackHours: 4,
      maxConversations: 8,
      allowCutOut: false,
      allowCutIn: true,
    },
    accountCount: 2,
    groupCount: 2,
    updatedAt: '2026-03-14T12:00:00.000Z',
  },
]

function createDetail(summary: TagSummary): TagDetail {
  return { ...summary }
}

function createHarness(options?: {
  initialSelectedTagIds?: number[]
  pageCreatedTagIds?: number[]
  onChangeSpy?: (tagIds: number[]) => void
  onCreateTagSpy?: (payload: CreateTagPayload) => void
  onUpdateTagSpy?: (tagId: number, payload: UpdateTagPayload) => void
  onDeleteTagSpy?: (tagId: number) => void
}) {
  const {
    initialSelectedTagIds = [],
    pageCreatedTagIds = [],
    onChangeSpy,
    onCreateTagSpy,
    onUpdateTagSpy,
    onDeleteTagSpy,
  } = options ?? {}

  return function Harness() {
    const [items, setItems] = React.useState<TagSummary[]>(tags)
    const [selectedTagIds, setSelectedTagIds] = React.useState<number[]>(initialSelectedTagIds)

    const onChange = (nextTagIds: number[]) => {
      onChangeSpy?.(nextTagIds)
      setSelectedTagIds(nextTagIds)
    }

    const onCreateTag = async (payload: CreateTagPayload) => {
      onCreateTagSpy?.(payload)
      const detail: TagDetail = {
        id: 99,
        name: payload.name,
        routingRule: {
          guardEnabled: payload.guardEnabled,
          lookbackHours: payload.lookbackHours ?? null,
          maxConversations: payload.maxConversations ?? null,
          allowCutOut: payload.allowCutOut,
          allowCutIn: payload.allowCutIn,
        },
        accountCount: 0,
        groupCount: 0,
        updatedAt: '2026-03-18T12:00:00.000Z',
      }
      setItems((current) => [...current, detail])
      return detail
    }

    const onUpdateTag = async (tagId: number, payload: UpdateTagPayload) => {
      onUpdateTagSpy?.(tagId, payload)
      const current = items.find((item) => item.id === tagId) ?? tags[0]!
      return createDetail({
        ...current,
        name: payload.name ?? current.name,
        routingRule: {
          guardEnabled: payload.guardEnabled ?? current.routingRule.guardEnabled,
          lookbackHours: payload.lookbackHours ?? current.routingRule.lookbackHours,
          maxConversations: payload.maxConversations ?? current.routingRule.maxConversations,
          allowCutOut: payload.allowCutOut ?? current.routingRule.allowCutOut,
          allowCutIn: payload.allowCutIn ?? current.routingRule.allowCutIn,
        },
      })
    }

    const onDeleteTag = async (tagId: number) => {
      onDeleteTagSpy?.(tagId)
      setItems((current) => current.filter((item) => item.id !== tagId))
      setSelectedTagIds((current) => current.filter((value) => value !== tagId))
    }

    return (
      <AccountTagField
        tags={items}
        selectedTagIds={selectedTagIds}
        writesEnabled
        pageCreatedTagIds={pageCreatedTagIds}
        labels={labels}
        onChange={onChange}
        onCreateTag={onCreateTag}
        onUpdateTag={onUpdateTag}
        onDeleteTag={onDeleteTag}
      />
    )
  }
}

describe('AccountTagField', () => {
  it('renders empty state inline and keeps the popover open while toggling multiple tags', () => {
    const onChangeSpy = vi.fn()
    const Harness = createHarness({ onChangeSpy })
    render(<Harness />)

    expect(document.body.textContent).toContain('No tag selected yet.')
    expect(document.body.querySelector('.border-dashed')).toBeNull()

    const addButton = document.querySelector('button[aria-label="Add tag"]') as HTMLButtonElement
    act(() => {
      addButton.click()
    })

    expect(document.body.textContent).toContain('vip-routing')
    expect(document.body.textContent).toContain('handoff-blocked')

    const vipOption = Array.from(document.querySelectorAll('[cmdk-item]')).find((node) => node.textContent?.includes('vip-routing')) as HTMLElement
    const handoffOption = Array.from(document.querySelectorAll('[cmdk-item]')).find((node) => node.textContent?.includes('handoff-blocked')) as HTMLElement

    act(() => {
      vipOption.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expect(onChangeSpy).toHaveBeenLastCalledWith([1])
    expect(document.body.querySelector('[cmdk-list]')).not.toBeNull()

    act(() => {
      handoffOption.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    expect(onChangeSpy).toHaveBeenLastCalledWith([1, 2])
    expect(document.body.querySelector('[cmdk-list]')).not.toBeNull()
  })

  it('filters tags and hands the current search text into create flow', async () => {
    const onCreateTagSpy = vi.fn()
    const Harness = createHarness({ onCreateTagSpy })
    render(<Harness />)

    const addButton = document.querySelector('button[aria-label="Add tag"]') as HTMLButtonElement
    act(() => {
      addButton.click()
    })

    const searchInput = document.querySelector('input[placeholder="Search existing tags..."]') as HTMLInputElement
    act(() => {
      const valueSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')?.set
      valueSetter?.call(searchInput, 'night-shift')
      searchInput.dispatchEvent(new Event('input', { bubbles: true }))
      searchInput.dispatchEvent(new Event('change', { bubbles: true }))
    })

    expect(document.body.textContent).toContain('Create "night-shift"')

    const createOption = Array.from(document.querySelectorAll('[cmdk-item]')).find((node) => node.textContent?.includes('Create "night-shift"')) as HTMLElement
    act(() => {
      createOption.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const nameInput = document.querySelector('input[name="tagName"]') as HTMLInputElement
    expect(nameInput.value).toBe('night-shift')

    const createButton = Array.from(document.querySelectorAll('button')).find((node) => node.textContent?.trim() === 'Create') as HTMLButtonElement
    await act(async () => {
      createButton.click()
    })

    expect(onCreateTagSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'night-shift',
      }),
    )
    expect(document.body.textContent).toContain('night-shift')
    expect(document.querySelector('input[name="tagName"]')).toBeNull()
  })

  it('preserves chip actions inside the inline picker', async () => {
    const onDeleteTagSpy = vi.fn()
    const Harness = createHarness({ initialSelectedTagIds: [1], pageCreatedTagIds: [1], onDeleteTagSpy })
    render(<Harness />)

    expect(document.body.textContent).toContain('vip-routing')

    const wrapper = document.querySelector('.relative.inline-flex') as HTMLElement
    act(() => {
      wrapper.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, relatedTarget: null }))
    })

    const actionButton = document.querySelector('button[aria-haspopup="menu"]') as HTMLButtonElement
    act(() => {
      actionButton.click()
    })

    const removeButton = Array.from(document.querySelectorAll('button')).find((node) => node.textContent?.includes('Delete and unlink')) as HTMLButtonElement
    await act(async () => {
      removeButton.click()
    })

    expect(onDeleteTagSpy).toHaveBeenCalledWith(1)
    expect(document.body.textContent).not.toContain('vip-routing')
  })
})
