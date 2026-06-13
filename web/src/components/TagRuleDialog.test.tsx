/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { TagRuleDialog } from './TagRuleDialog'
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

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(ui)
  })
}

function findButtonByText(text: string) {
  return Array.from(document.querySelectorAll('button')).find(
    (button) => button.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined
}

function findComboboxByLabel(label: string) {
  return Array.from(document.querySelectorAll('button[role="combobox"]')).find(
    (button) => button.getAttribute('aria-label') === label,
  ) as HTMLButtonElement | undefined
}

const labels = {
  createTitle: 'Create tag',
  editTitle: 'Edit tag',
  description: 'Configure routing rules.',
  name: 'Name',
  namePlaceholder: 'vip',
  blockNewConversations: 'Guard',
  forbidNewConversation: 'Block new conversations',
  allowCutOut: 'Cut out is not blocked',
  allowCutIn: 'Cut in is not blocked',
  forbidCutOut: 'Block cut out',
  forbidCutIn: 'Block cut in',
  priorityTier: 'Preferred usage',
  priorityPrimary: 'Primary',
  priorityNormal: 'Normal',
  priorityFallback: 'Fallback only',
  fastModeRewriteMode: 'Fast mode',
  fastModeKeepOriginal: 'Keep original',
  fastModeFillMissing: 'Fill when missing',
  fastModeForceAdd: 'Force add',
  fastModeForceRemove: 'Force remove',
  concurrencyLimit: 'Concurrency limit',
  concurrencyHint: 'Use 1-30. The last step means unlimited.',
  currentValue: 'Current',
  unlimited: 'Unlimited',
  availableModels: 'Available models',
  availableModelsHint: 'Leave empty to inherit.',
  availableModelsSearchPlaceholder: 'Search models',
  availableModelsEmpty: 'No matching models',
  availableModelsAll: 'Inherited / unrestricted',
  availableModelsCustomLabel: (value: string) => value,
  availableModelsAddCustom: 'Add custom model id',
  availableModelsInherited: 'Clear and inherit',
  availableModelsRemove: 'Remove model',
  cancel: 'Cancel',
  save: 'Save',
  create: 'Create',
  validation: 'Review the routing policy before saving.',
}

describe('TagRuleDialog', () => {
  it('submits unlimited as the default create payload', async () => {
    const onSubmit = vi.fn()
    render(
      <TagRuleDialog
        open
        mode="create"
        draftName="vip"
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Create',
    )
    expect(submit).not.toBeNull()

    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'vip',
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        concurrencyLimit: 0,
      }),
    )
  })

  it('submits finite limits unchanged in edit mode', async () => {
    const onSubmit = vi.fn()
    const tag: TagSummary = {
      id: 11,
      name: 'lane',
      routingRule: {
        blockNewConversations: false,
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: 'primary',
        fastModeRewriteMode: 'force_remove',
        concurrencyLimit: 6,
      },
      accountCount: 0,
      groupCount: 0,
      updatedAt: '2026-03-31T00:00:00.000Z',
    }
    render(
      <TagRuleDialog
        open
        mode="edit"
        tag={tag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save',
    )

    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'lane',
        priorityTier: 'primary',
        fastModeRewriteMode: 'force_remove',
        concurrencyLimit: 6,
      }),
    )
  })

  it('submits policy-only payloads without a tag name', async () => {
    const onSubmit = vi.fn()
    render(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={null}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    expect(document.querySelector('input[name="tagName"]')).toBeNull()
    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save account policy',
    )

    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith(
      expect.not.objectContaining({
        name: expect.anything(),
      }),
    )
    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        concurrencyLimit: 0,
      }),
    )
  })

  it('can submit only changed policy fields for inherited account policy edits', async () => {
    const onSubmit = vi.fn()
    const tag: TagSummary = {
      id: 42,
      name: 'account@example.com',
      routingRule: {
        blockNewConversations: true,
        allowCutOut: true,
        allowCutIn: false,
        priorityTier: 'fallback',
        fastModeRewriteMode: 'force_add',
        concurrencyLimit: 4,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 3,
      },
      accountCount: 1,
      groupCount: 1,
      updatedAt: '2026-03-31T00:00:00.000Z',
    }
    render(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={tag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const switches = Array.from(document.querySelectorAll('button[role="switch"]'))
    act(() => {
      switches[1]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save account policy',
    )

    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith({
      allowCutOut: false,
    })
  })

  it('keeps an open policy draft when the same target is refreshed', async () => {
    const onSubmit = vi.fn()
    const originalTag: TagSummary = {
      id: 42,
      name: 'account@example.com',
      routingRule: {
        blockNewConversations: false,
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        concurrencyLimit: 0,
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
      },
      accountCount: 1,
      groupCount: 1,
      updatedAt: '2026-03-31T00:00:00.000Z',
    }
    render(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={originalTag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const switches = Array.from(document.querySelectorAll('button[role="switch"]'))
    act(() => {
      switches[1]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    rerender(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={{
          ...originalTag,
          name: 'account refreshed@example.com',
          routingRule: {
            ...originalTag.routingRule,
            priorityTier: 'primary',
            concurrencyLimit: 6,
          },
          updatedAt: '2026-04-01T00:00:00.000Z',
        }}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save account policy',
    )
    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith({
      allowCutOut: false,
    })
  })

  it('reinitializes the policy draft when the target changes while open', async () => {
    const onSubmit = vi.fn()
    const firstTag: TagSummary = {
      id: 42,
      name: 'first@example.com',
      routingRule: {
        blockNewConversations: false,
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        concurrencyLimit: 0,
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
      },
      accountCount: 1,
      groupCount: 1,
      updatedAt: '2026-03-31T00:00:00.000Z',
    }
    const secondTag: TagSummary = {
      ...firstTag,
      id: 43,
      name: 'second@example.com',
      routingRule: {
        ...firstTag.routingRule,
        allowCutOut: false,
      },
    }
    render(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={firstTag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const switches = Array.from(document.querySelectorAll('button[role="switch"]'))
    act(() => {
      switches[1]?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    rerender(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={secondTag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save account policy',
    )
    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith({})
  })

  it('serializes available models with dedupe when adding a custom model', async () => {
    const onSubmit = vi.fn()
    render(
      <TagRuleDialog
        open
        mode="create"
        draftName="vip"
        availableModelOptions={['gpt-5.5', 'gpt-5.4-mini']}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const input = document.querySelector('input[name="availableModelInput"]') as HTMLInputElement | null
    expect(input).not.toBeNull()
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      'value',
    )?.set
    act(() => {
      valueSetter?.call(input, 'gpt-5.5')
      input!.dispatchEvent(new Event('input', { bubbles: true }))
      input!.dispatchEvent(new Event('change', { bubbles: true }))
    })

    const addButton = findButtonByText('Add custom model id')
    expect(addButton).not.toBeNull()
    act(() => {
      addButton!.click()
    })
    act(() => {
      valueSetter?.call(input, 'gpt-5.5')
      input!.dispatchEvent(new Event('input', { bubbles: true }))
      input!.dispatchEvent(new Event('change', { bubbles: true }))
    })
    act(() => {
      addButton!.click()
    })

    const submit = findButtonByText('Create')
    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'vip',
        availableModels: ['gpt-5.5'],
      }),
    )
  })

  it('submits an empty availableModels list when changed-fields-only clears inheritance', async () => {
    const onSubmit = vi.fn()
    const tag: TagSummary = {
      id: 91,
      name: 'account@example.com',
      routingRule: {
        blockNewConversations: false,
        allowCutOut: true,
        allowCutIn: true,
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
        concurrencyLimit: 0,
        upstream429RetryEnabled: false,
        upstream429MaxRetries: 0,
        availableModels: ['gpt-5.5'],
      },
      accountCount: 1,
      groupCount: 0,
      updatedAt: '2026-04-01T00:00:00.000Z',
    }
    render(
      <TagRuleDialog
        open
        mode="edit"
        policyOnly
        changedFieldsOnly
        title="Account routing policy"
        submitLabel="Save account policy"
        tag={tag}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    const trigger = findComboboxByLabel('Available models')
    expect(trigger).toBeTruthy()
    act(() => {
      trigger!.click()
    })

    const clearAfterOpen = Array.from(document.querySelectorAll('[role="option"], [cmdk-item], button')).find(
      (element) => element.textContent?.trim() === 'Clear and inherit',
    ) as HTMLElement | undefined
    expect(clearAfterOpen).toBeTruthy()
    act(() => {
      clearAfterOpen!.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Save account policy',
    )
    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith({
      availableModels: [],
    })
  })
})
