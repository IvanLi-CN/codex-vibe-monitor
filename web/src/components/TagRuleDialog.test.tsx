/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { TagRuleDialog } from './TagRuleDialog'
import type { TagSummary } from '../lib/api'

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
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
})
