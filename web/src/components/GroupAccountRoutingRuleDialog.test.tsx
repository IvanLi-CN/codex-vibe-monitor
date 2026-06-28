/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { GroupAccountRoutingRule } from '../lib/api'
import { GroupAccountRoutingRuleDialog } from './GroupAccountRoutingRuleDialog'

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
  allowNewConversations: 'New conversations',
  newConversationHint: 'Allow new conversations on this group',
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
  imageToolRewriteMode: 'Image tools',
  imageToolKeepOriginal: 'Keep original',
  imageToolFillMissing: 'Fill when missing',
  imageToolForceAdd: 'Force add',
  imageToolForceRemove: 'Force remove',
  imageToolRewriteHint:
    "Keep original follows the account's own image capability. Fill when missing only injects image tools when image intent is confirmed; force add always injects; force remove always strips it.",
  concurrencyLimit: 'Concurrency limit',
  concurrencyHint: 'Use 1-30 to cap fresh assignments. The last slider step means unlimited.',
  currentValue: 'Current',
  unlimited: 'Unlimited',
  availableModels: 'Available models',
  availableModelsHint: 'Leave empty to inherit. Automatic and sticky routing only consider matching accounts.',
  availableModelsSearchPlaceholder: 'Search models',
  availableModelsEmpty: 'No matching models',
  availableModelsAll: 'Inherited / unrestricted',
  availableModelsCustomLabel: (value: string) => value,
  availableModelsAddCustom: 'Add custom model id',
  availableModelsInherited: 'Clear and inherit',
  availableModelsRemove: 'Remove model',
  upstream429Retry: 'Upstream 429 retry',
  upstream429RetryHint: 'Retry the same upstream account before cooldown and failover.',
  upstream429RetryToggle: 'Retry after upstream 429',
  upstream429RetryCount: 'Retry count',
  upstream429RetryCountOnce: '1 retry',
  upstream429RetryCountMany: (count: number) => `${count} retries`,
  cancel: 'Cancel',
  validation: 'Review the routing policy before saving.',
}

const defaultRule: GroupAccountRoutingRule = {
  blockNewConversations: false,
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: 'normal',
  fastModeRewriteMode: 'keep_original',
  imageToolRewriteMode: 'keep_original',
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  availableModels: [],
}

describe('GroupAccountRoutingRuleDialog', () => {
  it('submits the default image tool rewrite mode', () => {
    const onSubmit = vi.fn()
    render(
      <GroupAccountRoutingRuleDialog
        open
        title="Group policy"
        description="Shared routing policy"
        submitLabel="Apply group policy"
        rule={defaultRule}
        onClose={() => undefined}
        onSubmit={onSubmit}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('Image tools')
    const submit = Array.from(document.querySelectorAll('button')).find(
      (button) => button.textContent?.trim() === 'Apply group policy',
    )
    expect(submit).toBeInstanceOf(HTMLButtonElement)

    act(() => {
      submit!.click()
    })

    expect(onSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        imageToolRewriteMode: 'keep_original',
        priorityTier: 'normal',
        fastModeRewriteMode: 'keep_original',
      }),
    )
    expect(onSubmit).toHaveBeenCalledWith(
      expect.not.objectContaining({
        availableModels: [],
      }),
    )
  })
})
