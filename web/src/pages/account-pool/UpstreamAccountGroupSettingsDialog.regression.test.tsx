/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { I18nProvider } from '../../i18n'
import { useUpstreamAccountGroupSettingsDialog } from './useUpstreamAccountGroupSettingsDialog'
import { normalizeGroupName } from '../../lib/upstreamAccountGroups'

const hookMocks = vi.hoisted(() => ({
  useForwardProxyBindingNodes: vi.fn(),
}))

vi.mock('../../hooks/useForwardProxyBindingNodes', () => ({
  useForwardProxyBindingNodes: hookMocks.useForwardProxyBindingNodes,
}))

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
  hookMocks.useForwardProxyBindingNodes.mockReset()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<I18nProvider>{ui}</I18nProvider>)
  })
}

function pressButton(button: HTMLButtonElement) {
  act(() => {
    button.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))
    button.dispatchEvent(new PointerEvent('pointerup', { bubbles: true }))
    button.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    button.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
}

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function findGroupSettingsDialog() {
  return Array.from(document.body.querySelectorAll('[role="dialog"]')).find(
    (dialog) =>
      Array.from(dialog.querySelectorAll('h1, h2, h3, [role="heading"]')).some(
        (candidate) =>
          /^(group settings|分组设置)$/i.test(
            candidate.textContent?.trim() ?? '',
          ),
      ),
  ) as HTMLElement | undefined
}

function findDialogByHeading(pattern: RegExp) {
  return Array.from(document.body.querySelectorAll('[role="dialog"]')).find(
    (dialog) =>
      Array.from(dialog.querySelectorAll('h1, h2, h3, [role="heading"]')).some(
        (candidate) => pattern.test(candidate.textContent?.trim() ?? ''),
      ),
  ) as HTMLElement | undefined
}

function findButtonByPattern(
  pattern: RegExp,
  root: ParentNode = document.body,
) {
  return Array.from(root.querySelectorAll('button')).find((candidate) =>
    pattern.test(candidate.textContent ?? ''),
  ) as HTMLButtonElement | undefined
}

function deleteButtonFromOpenDialog() {
  const dialog = findGroupSettingsDialog()
  expect(dialog).toBeTruthy()
  const deleteButton = Array.from(dialog?.querySelectorAll('button') ?? []).find(
    (candidate) => /^delete$|删除/i.test(candidate.textContent ?? ''),
  )
  expect(deleteButton).toBeTruthy()
  return deleteButton as HTMLButtonElement
}

function openButtonByLabel(label: string) {
  const button = Array.from(document.body.querySelectorAll('button')).find(
    (candidate) => candidate.textContent?.trim() === label,
  )
  expect(button).toBeInstanceOf(HTMLButtonElement)
  return button as HTMLButtonElement
}

function readValue(testId: string) {
  const input = document.querySelector(`[data-testid="${testId}"]`)
  expect(input).toBeInstanceOf(HTMLInputElement)
  return (input as HTMLInputElement).value
}

function createGroupState(groupName: string) {
  return {
    groupName,
    existing: true,
    accountCount: 0,
    note: 'prod note',
    boundProxyKeys: [],
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    concurrencyLimit: 0,
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
  }
}

function DetailDeleteHarness({
  deleteGroupNote,
}: {
  deleteGroupNote: (groupName: string) => Promise<void>
}) {
  const [draftGroupName, setDraftGroupName] = React.useState('prod')
  const { openEditor, dialog } = useUpstreamAccountGroupSettingsDialog({
    writesEnabled: true,
    resolveGroupState: (groupName) => createGroupState(groupName),
    saveGroupSettings: async () => undefined,
    deleteGroupSettings: async (groupName) => {
      await deleteGroupNote(groupName)
      setDraftGroupName((current) =>
        normalizeGroupName(current) === normalizeGroupName(groupName) ? '' : current,
      )
    },
  })

  return (
    <>
      <input data-testid="detail-group" readOnly value={draftGroupName} />
      <button type="button" onClick={() => openEditor(draftGroupName)}>
        Open detail group settings
      </button>
      {dialog}
    </>
  )
}

function BulkDeleteHarness({
  deleteGroupNote,
}: {
  deleteGroupNote: (groupName: string) => Promise<void>
}) {
  const [bulkGroupName, setBulkGroupName] = React.useState('prod')
  const { openEditor, dialog } = useUpstreamAccountGroupSettingsDialog({
    writesEnabled: true,
    resolveGroupState: (groupName) => createGroupState(groupName),
    saveGroupSettings: async () => undefined,
    deleteGroupSettings: async (groupName) => {
      await deleteGroupNote(groupName)
      if (normalizeGroupName(bulkGroupName) === normalizeGroupName(groupName)) {
        setBulkGroupName('')
      }
    },
  })

  return (
    <>
      <input data-testid="bulk-group" readOnly value={bulkGroupName} />
      <button type="button" onClick={() => openEditor(bulkGroupName)}>
        Open bulk group settings
      </button>
      {dialog}
    </>
  )
}

describe('useUpstreamAccountGroupSettingsDialog regression', () => {
  beforeEach(() => {
    hookMocks.useForwardProxyBindingNodes.mockReturnValue({
      nodes: [],
      error: null,
      isLoading: false,
      refresh: vi.fn(),
      catalogState: {
        kind: 'ready-empty',
        freshness: 'fresh',
        isPending: false,
        hasNodes: false,
      },
    })
  })

  it('clears the detail draft field after deleting the active group', async () => {
    const deleteGroupNote = vi.fn().mockResolvedValue(undefined)
    render(<DetailDeleteHarness deleteGroupNote={deleteGroupNote} />)

    expect(readValue('detail-group')).toBe('prod')
    pressButton(openButtonByLabel('Open detail group settings'))
    await flushAsync()

    pressButton(deleteButtonFromOpenDialog())
    await flushAsync()

    expect(deleteGroupNote).toHaveBeenCalledWith('prod')
    expect(readValue('detail-group')).toBe('')
  })

  it('clears the bulk draft field after deleting the active group', async () => {
    const deleteGroupNote = vi.fn().mockResolvedValue(undefined)
    render(<BulkDeleteHarness deleteGroupNote={deleteGroupNote} />)

    expect(readValue('bulk-group')).toBe('prod')
    pressButton(openButtonByLabel('Open bulk group settings'))
    await flushAsync()

    pressButton(deleteButtonFromOpenDialog())
    await flushAsync()

    expect(deleteGroupNote).toHaveBeenCalledWith('prod')
    expect(readValue('bulk-group')).toBe('')
  })

  it('preserves cleared availableModels in the group routing policy payload', async () => {
    const saveGroupSettings = vi.fn().mockResolvedValue(undefined)

    function Harness() {
      const { openEditor, dialog } = useUpstreamAccountGroupSettingsDialog({
        writesEnabled: true,
        resolveGroupState: (groupName) => createGroupState(groupName),
        saveGroupSettings,
      })

      return (
        <>
          <button type="button" onClick={() => openEditor('prod')}>
            Open group settings
          </button>
          {dialog}
        </>
      )
    }

    render(<Harness />)
    pressButton(openButtonByLabel('Open group settings'))
    await flushAsync()

    const groupDialog = findGroupSettingsDialog()
    expect(groupDialog).toBeTruthy()
    const editRoutingPolicy = findButtonByPattern(
      /edit policy|编辑策略/i,
      groupDialog!,
    )
    expect(editRoutingPolicy).toBeTruthy()
    pressButton(editRoutingPolicy!)
    await flushAsync()

    const routingPolicyDialog = findDialogByHeading(
      /group routing policy|分组路由策略/i,
    )
    expect(routingPolicyDialog).toBeTruthy()
    const availableModelsTrigger = routingPolicyDialog?.querySelector(
      'button[role="combobox"][aria-label="Available models"], button[role="combobox"][aria-label="可用模型"]',
    ) as HTMLButtonElement | null
    expect(availableModelsTrigger).toBeTruthy()
    pressButton(availableModelsTrigger!)
    await flushAsync()

    const clearAndInherit = Array.from(
      document.querySelectorAll('[role="option"], [cmdk-item], button'),
    ).find((candidate) =>
      /clear and inherit|清空并继承/i.test(candidate.textContent ?? ''),
    ) as HTMLElement | undefined
    expect(clearAndInherit).toBeTruthy()
    act(() => {
      clearAndInherit?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })
    await flushAsync()

    const savePolicy = findButtonByPattern(
      /apply group policy|应用分组策略/i,
      routingPolicyDialog!,
    )
    expect(savePolicy).toBeTruthy()
    pressButton(savePolicy!)
    await flushAsync()

    const refreshedGroupDialog = findGroupSettingsDialog()
    expect(refreshedGroupDialog).toBeTruthy()
    const saveGroup = findButtonByPattern(
      /save changes|保存修改/i,
      refreshedGroupDialog!,
    )
    expect(saveGroup).toBeTruthy()
    pressButton(saveGroup!)
    await flushAsync()

    expect(saveGroupSettings).toHaveBeenCalledWith(
      'prod',
      expect.objectContaining({
        routingRule: expect.objectContaining({
          availableModels: [],
        }),
      }),
      { existing: true },
    )
  })
})
