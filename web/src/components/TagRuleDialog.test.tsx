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

const labels = {
  createTitle: 'Create tag',
  editTitle: 'Edit tag',
  description: 'Configure routing rules.',
  name: 'Name',
  namePlaceholder: 'vip',
  guardEnabled: 'Guard',
  lookbackHours: 'Lookback',
  maxConversations: 'Max conversations',
  allowCutOut: 'Allow cut out',
  allowCutIn: 'Allow cut in',
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
  validation: 'Guard inputs must be positive integers.',
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
        guardEnabled: false,
        lookbackHours: null,
        maxConversations: null,
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
})
