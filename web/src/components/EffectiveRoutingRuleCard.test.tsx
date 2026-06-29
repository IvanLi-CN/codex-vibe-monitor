/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

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
  title: 'Effective routing rule',
  description: 'Merged routing constraints applied to the selected upstream account. Use account overrides when needed.',
  noTags: 'No tags linked',
  blockNewConversations: 'New conversations blocked',
  allowNewConversations: 'New conversations allowed',
  allowCutOut: 'Cut-out allowed',
  denyCutOut: 'Cut-out blocked',
  allowCutIn: 'Cut-in allowed',
  denyCutIn: 'Cut-in blocked',
  sourceTags: 'Source tags',
  priorityPrimary: 'Primary',
  priorityNormal: 'Normal',
  priorityFallback: 'Fallback only',
  fastModeKeepOriginal: 'Keep original',
  fastModeFillMissing: 'Fill when missing',
  fastModeForceAdd: 'Force add',
  fastModeForceRemove: 'Force remove',
  imageToolKeepOriginal: 'Keep original',
  imageToolFillMissing: 'Fill when missing',
  imageToolForceAdd: 'Force add',
  imageToolForceRemove: 'Force remove',
  availableModelsInherited: 'Inherited / unrestricted',
  availableModelsNoneAllowed: 'No models allowed',
  availableModelsEmpty: 'No matching models',
  systemDeniedModelsEmpty: 'None',
  concurrencyLimit: (count: number) => `Concurrency ${count}`,
  concurrencyUnlimited: 'Concurrency unlimited',
  sourceBreakdownTitle: 'Field source breakdown',
  fieldBlockNewConversations: 'New conversations',
  fieldAllowCutOut: 'Cut out',
  fieldAllowCutIn: 'Cut in',
  fieldPriority: 'Priority',
  fieldFastMode: 'FAST mode',
  fieldImageToolRewriteMode: 'Image tools',
  fieldConcurrency: 'Concurrency',
  fieldUpstream429: 'Upstream 429 retry',
  fieldAvailableModels: 'Available models',
  fieldSystemDeniedModels: 'System denied models',
  sourceRoot: 'Root default',
  sourceGroup: 'Group',
  sourceTag: 'Tag',
  sourceAccount: 'Account',
  sourceSystem: 'System',
  overrideEdit: 'Edit account override',
  overrideClear: 'Clear account override',
  overrideSaving: 'Saving account override...',
  newConversationLabel: 'New conversations',
  cutOutLabel: 'Cut out',
  cutInLabel: 'Cut in',
  upstream429RetryCountValue: (count: number) => String(count),
  currentValue: 'Current value',
  availableModelsAddCustom: 'Add model',
  availableModelsCustomLabel: (value: string) => value,
  availableModelsRemove: 'Remove model',
  availableModelsPlaceholder: 'Model id',
}

function buildRule(overrides: Partial<EffectiveRoutingRule> = {}): EffectiveRoutingRule {
  return {
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
    systemDeniedModels: [],
    sourceTagIds: [],
    sourceTagNames: [],
    fieldSources: {
      blockNewConversations: 'root',
      allowCutOut: 'root',
      allowCutIn: 'root',
      priorityTier: 'root',
      fastModeRewriteMode: 'root',
      imageToolRewriteMode: 'root',
      concurrencyLimit: 'root',
      upstream429Retry: 'root',
      availableModels: 'root',
      systemDeniedModels: 'root',
    },
    ...overrides,
  }
}

describe('EffectiveRoutingRuleCard', () => {
  it('shows inherited copy when no available model constraint is defined', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule()}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('Inherited / unrestricted')
    expect(document.body.textContent).toContain('Image tools')
    expect(document.body.textContent).not.toContain('No models allowed')
  })

  it('shows deny-all copy for empty tag intersections', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: [],
          sourceTagIds: [1, 2],
          sourceTagNames: ['allow-gpt-4o', 'allow-o3'],
          fieldSources: {
            blockNewConversations: 'root',
            allowCutOut: 'root',
            allowCutIn: 'root',
            priorityTier: 'tag',
            fastModeRewriteMode: 'tag',
            concurrencyLimit: 'tag',
            upstream429Retry: 'root',
            availableModels: 'tag',
            systemDeniedModels: 'root',
          },
        })}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('No models allowed')
    expect(document.body.textContent).not.toContain('Inherited / unrestricted')
  })

  it('shows deny-all copy for empty group model overrides', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: [],
          fieldSources: {
            ...buildRule().fieldSources,
            availableModels: 'group',
          },
        })}
        labels={labels}
      />,
    )

    expect(document.body.textContent).toContain('No models allowed')
    expect(document.body.textContent).not.toContain('Inherited / unrestricted')
  })

  it('expands an inherited boolean override and saves the positive switch as the backend inverse', () => {
    const onChange = vi.fn()
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule()}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    )

    const editButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: New conversations"]',
    )
    expect(editButton).not.toBeNull()

    act(() => {
      editButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(document.body.textContent).not.toContain('Default value starts from the inherited value.')
    expect(document.body.textContent).toContain('New conversationsNew conversations allowedRoot defaultNew conversations')
    const switchButton = document.querySelector<HTMLButtonElement>('button[role="switch"][aria-label="New conversations"]')
    expect(switchButton).not.toBeNull()

    act(() => {
      switchButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onChange).toHaveBeenCalledWith('allowNewConversations', { allowNewConversations: false })
  })

  it('clears an account override when the active override button is clicked again', () => {
    const onChange = vi.fn()
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          allowCutIn: false,
          fieldSources: {
            ...buildRule().fieldSources,
            allowCutIn: 'account',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    )

    const clearButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Clear account override: Cut in"]',
    )
    expect(clearButton).not.toBeNull()

    act(() => {
      clearButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onChange).toHaveBeenCalledWith('allowCutIn', { allowCutIn: null })
  })

  it('expands the first account override by default', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          fastModeRewriteMode: 'force_add',
          fieldSources: {
            ...buildRule().fieldSources,
            fastModeRewriteMode: 'account',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    )

    expect(document.body.textContent).not.toContain('Default value starts from the inherited value.')
    expect(document.body.textContent).toContain('FAST modeForce addAccountFAST mode')
    const activeButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Clear account override: FAST mode"]',
    )
    expect(activeButton?.getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull()
  })

  it('expands every existing account override by default', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          allowCutOut: false,
          allowCutIn: false,
          priorityTier: 'primary',
          fastModeRewriteMode: 'force_add',
          concurrencyLimit: 3,
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 5,
          fieldSources: {
            ...buildRule().fieldSources,
            allowCutOut: 'account',
            allowCutIn: 'account',
            priorityTier: 'account',
            fastModeRewriteMode: 'account',
            concurrencyLimit: 'account',
            upstream429Retry: 'account',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    )

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull()
    expect(document.querySelector('[role="switch"][aria-label="Cut in"]')).not.toBeNull()
    expect(document.querySelector('[role="radiogroup"][aria-label="Priority"]')).not.toBeNull()
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull()
    expect(document.querySelector('[role="radiogroup"][aria-label="Upstream 429 retry"]')).not.toBeNull()
    expect(document.querySelector('button[role="switch"][aria-label="Upstream 429 retry"]')).toBeNull()
    expect(document.body.textContent).toContain('Concurrency 3')
    expect(document.body.textContent).toContain('5')
    expect(document.body.textContent).not.toContain('Account override')
    expect(document.body.textContent).not.toContain('Default value starts from the inherited value.')
    expect(document.body.textContent).toContain('Cut outCut-out blockedAccountCut out')
    expect(document.body.textContent).toContain('FAST modeForce addAccountFAST mode')
  })

  it('saves 429 retry as a single 0..5 count selector', () => {
    const onChange = vi.fn()
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          upstream429RetryEnabled: true,
          upstream429MaxRetries: 4,
          fieldSources: {
            ...buildRule().fieldSources,
            upstream429Retry: 'account',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange }}
      />,
    )

    const zeroButton = document.querySelector<HTMLButtonElement>(
      '[role="radiogroup"][aria-label="Upstream 429 retry"] button[role="radio"]',
    )
    expect(zeroButton?.textContent).toBe('0')

    act(() => {
      zeroButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(onChange).toHaveBeenCalledWith('upstream429Retry', {
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
    })
  })

  it('renders available models override as a tag selector trigger with selected chips', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          availableModels: ['gpt-5.5', 'gpt-5.4-mini'],
          fieldSources: {
            ...buildRule().fieldSources,
            availableModels: 'account',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    )

    const trigger = document.querySelector<HTMLButtonElement>(
      'button[role="combobox"][aria-label="Available models"]',
    )
    expect(trigger).not.toBeNull()
    expect(trigger?.textContent).toContain('gpt-5.5')
    expect(trigger?.textContent).toContain('gpt-5.4-mini')
    expect(document.body.textContent).not.toContain('Add gpt-5.5')
    expect(document.body.textContent).not.toContain('Add gpt-5.4-mini')
  })

  it('keeps a user-opened inherited field when editable policy identity changes', () => {
    const rule = buildRule({
      fastModeRewriteMode: 'force_add',
      fieldSources: {
        ...buildRule().fieldSources,
        fastModeRewriteMode: 'account',
      },
    })
    const onChange = vi.fn()

    render(
        <EffectiveRoutingRuleCard
          rule={rule}
          identityKey="account-a"
          labels={labels}
          editablePolicy={{ onChange }}
        />,
    )

    const cutOutButton = document.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Cut out"]',
    )
    expect(cutOutButton).not.toBeNull()

    act(() => {
      cutOutButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    })

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull()

    act(() => {
      root?.render(
        <EffectiveRoutingRuleCard
          rule={{
            ...rule,
            fieldSources: {
              ...rule.fieldSources,
            },
          }}
          identityKey="account-a"
          labels={labels}
          editablePolicy={{ onChange, busyField: null }}
        />,
      )
    })

    expect(document.querySelector('[role="switch"][aria-label="Cut out"]')).not.toBeNull()
    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull()

    act(() => {
      root?.render(
        <EffectiveRoutingRuleCard
          rule={{
            ...rule,
            fieldSources: {
              ...rule.fieldSources,
            },
          }}
          identityKey="account-b"
          labels={labels}
          editablePolicy={{ onChange }}
        />,
      )
    })

    expect(document.querySelector('[role="radiogroup"][aria-label="FAST mode"]')).not.toBeNull()
  })

  it('keeps system denied models read-only even when account policy editing is enabled', () => {
    render(
      <EffectiveRoutingRuleCard
        rule={buildRule({
          systemDeniedModels: ['gpt-5.5'],
          fieldSources: {
            ...buildRule().fieldSources,
            systemDeniedModels: 'system',
          },
        })}
        labels={labels}
        editablePolicy={{ onChange: vi.fn() }}
      />,
    )

    expect(document.body.textContent).toContain('gpt-5.5')
    expect(
      document.querySelector('button[aria-label="Edit account override: System denied models"]'),
    ).toBeNull()
  })
})
