import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent } from 'storybook/test'
import type { UpdateGroupAccountRoutingRulePayload } from '../lib/api'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

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
  overrideActive: 'Account override',
  overrideClear: 'Clear account override',
  overrideSaving: 'Saving account override...',
  inheritValue: 'Default value starts from the inherited value.',
  newConversationLabel: 'New conversations',
  cutOutLabel: 'Cut out',
  cutInLabel: 'Cut in',
  upstream429RetryCountValue: (count: number) => String(count),
  availableModelsAddCustom: 'Add model',
  availableModelsCustomLabel: (value: string) => value,
  availableModelsRemove: 'Remove model',
  availableModelsPlaceholder: 'Model id',
  currentValue: 'Current value',
}

const relaxedRule: EffectiveRoutingRule = {
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
  timeouts: {
    responsesFirstByteTimeoutSecs: 120,
    compactFirstByteTimeoutSecs: 300,
    responsesStreamTimeoutSecs: 300,
    compactStreamTimeoutSecs: 300,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: 'root',
    compactFirstByteTimeoutSecs: 'root',
    responsesStreamTimeoutSecs: 'root',
    compactStreamTimeoutSecs: 'root',
  },
}

const strictRule: EffectiveRoutingRule = {
  blockNewConversations: true,
  allowCutOut: false,
  allowCutIn: false,
  priorityTier: 'fallback',
  fastModeRewriteMode: 'force_remove',
  imageToolRewriteMode: 'force_add',
  concurrencyLimit: 2,
  upstream429RetryEnabled: true,
  upstream429MaxRetries: 4,
  availableModels: ['gpt-5.5', 'gpt-5.4-mini'],
  systemDeniedModels: ['gpt-5.5'],
  sourceTagIds: [1, 2],
  sourceTagNames: ['vip-routing', 'handoff-blocked'],
  fieldSources: {
    blockNewConversations: 'group',
    allowCutOut: 'tag',
    allowCutIn: 'account',
    priorityTier: 'tag',
    fastModeRewriteMode: 'account',
    imageToolRewriteMode: 'tag',
    concurrencyLimit: 'tag',
    upstream429Retry: 'account',
    availableModels: 'account',
    systemDeniedModels: 'system',
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 45,
    compactFirstByteTimeoutSecs: 300,
    responsesStreamTimeoutSecs: 210,
    compactStreamTimeoutSecs: 300,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: 'account',
    compactFirstByteTimeoutSecs: 'root',
    responsesStreamTimeoutSecs: 'account',
    compactStreamTimeoutSecs: 'group',
  },
}

const strictFieldSources = {
  blockNewConversations: 'group',
  allowCutOut: 'tag',
  allowCutIn: 'account',
  priorityTier: 'tag',
  fastModeRewriteMode: 'account',
  imageToolRewriteMode: 'account',
  concurrencyLimit: 'tag',
  upstream429Retry: 'account',
  availableModels: 'account',
  systemDeniedModels: 'system',
} as const

const denyAllTagIntersectionRule: EffectiveRoutingRule = {
  ...strictRule,
  availableModels: [],
  systemDeniedModels: [],
  sourceTagIds: [1, 2],
  sourceTagNames: ['allow-gpt-4o', 'allow-o3'],
  fieldSources: {
    ...strictFieldSources,
    availableModels: 'tag',
    systemDeniedModels: 'root',
  },
}

const multipleAccountOverridesRule: EffectiveRoutingRule = {
  ...strictRule,
  allowCutOut: false,
  allowCutIn: false,
  priorityTier: 'primary',
  fastModeRewriteMode: 'force_add',
  concurrencyLimit: 3,
  upstream429RetryEnabled: true,
  upstream429MaxRetries: 5,
  fieldSources: {
    blockNewConversations: strictRule.fieldSources?.blockNewConversations ?? 'root',
    allowCutOut: 'account',
    allowCutIn: 'account',
    priorityTier: 'account',
    fastModeRewriteMode: 'account',
    imageToolRewriteMode: strictRule.fieldSources?.imageToolRewriteMode ?? 'root',
    concurrencyLimit: 'account',
    upstream429Retry: 'account',
    availableModels: strictRule.fieldSources?.availableModels ?? 'root',
    systemDeniedModels: strictRule.fieldSources?.systemDeniedModels ?? 'root',
  },
  timeouts: {
    responsesFirstByteTimeoutSecs: 30,
    compactFirstByteTimeoutSecs: 180,
    responsesStreamTimeoutSecs: 180,
    compactStreamTimeoutSecs: 240,
  },
  timeoutFieldSources: {
    responsesFirstByteTimeoutSecs: 'account',
    compactFirstByteTimeoutSecs: 'account',
    responsesStreamTimeoutSecs: 'account',
    compactStreamTimeoutSecs: 'group',
  },
}

const meta = {
  title: 'Account Pool/Components/Effective Routing Rule Card',
  component: EffectiveRoutingRuleCard,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
    docs: {
      description: {
        component:
          '账号详情页里的最终生效规则卡片。服务端已经合并多个 tag 规则，前端只负责展示最终约束与来源 tag。',
      },
    },
  },
  decorators: [
    (Story) => (
      <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
        <div className="mx-auto max-w-3xl">
          <Story />
        </div>
      </div>
    ),
  ],
  args: {
    labels,
    rule: relaxedRule,
  },
} satisfies Meta<typeof EffectiveRoutingRuleCard>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const StrictMergedRule: Story = {
  args: {
    rule: strictRule,
  },
}

export const DenyAllTagIntersection: Story = {
  args: {
    rule: denyAllTagIntersectionRule,
  },
}

export const PrimaryRule: Story = {
  args: {
    rule: {
      ...relaxedRule,
      priorityTier: 'primary',
      fastModeRewriteMode: 'force_add',
      sourceTagIds: [9],
      sourceTagNames: ['priority-lane'],
    },
  },
}

export const FillMissingRule: Story = {
  args: {
    rule: {
      ...relaxedRule,
      fastModeRewriteMode: 'fill_missing',
      sourceTagIds: [12],
      sourceTagNames: ['overflow-guard'],
    },
  },
}

const editableOptions = ['gpt-5.5', 'gpt-5.4-mini', 'o3', 'gpt-4.1']
type StoryFieldSources = NonNullable<EffectiveRoutingRule['fieldSources']>
type EditablePolicyConfig = NonNullable<Parameters<typeof EffectiveRoutingRuleCard>[0]['editablePolicy']>

function applyPatchToRule(rule: EffectiveRoutingRule, patch: UpdateGroupAccountRoutingRulePayload): EffectiveRoutingRule {
  const fieldSources: StoryFieldSources = {
    blockNewConversations: rule.fieldSources?.blockNewConversations ?? 'root',
    allowCutOut: rule.fieldSources?.allowCutOut ?? 'root',
    allowCutIn: rule.fieldSources?.allowCutIn ?? 'root',
    priorityTier: rule.fieldSources?.priorityTier ?? 'root',
    fastModeRewriteMode: rule.fieldSources?.fastModeRewriteMode ?? 'root',
    imageToolRewriteMode: rule.fieldSources?.imageToolRewriteMode ?? 'root',
    concurrencyLimit: rule.fieldSources?.concurrencyLimit ?? 'root',
    upstream429Retry: rule.fieldSources?.upstream429Retry ?? 'root',
    availableModels: rule.fieldSources?.availableModels ?? 'root',
    systemDeniedModels: rule.fieldSources?.systemDeniedModels ?? 'root',
  }
  const next: EffectiveRoutingRule = {
    ...rule,
    fieldSources,
  }
  const nextSources = fieldSources
  const sourceFor = (value: unknown): 'root' | 'account' => (value === null ? 'root' : 'account')
  if ('allowNewConversations' in patch || 'blockNewConversations' in patch) {
    const value =
      patch.allowNewConversations ??
      (typeof patch.blockNewConversations === 'boolean' ? !patch.blockNewConversations : patch.blockNewConversations)
    if (typeof value === 'boolean') next.blockNewConversations = !value
    nextSources.blockNewConversations = sourceFor(value)
  }
  if ('allowCutOut' in patch) {
    if (typeof patch.allowCutOut === 'boolean') next.allowCutOut = patch.allowCutOut
    nextSources.allowCutOut = sourceFor(patch.allowCutOut)
  }
  if ('allowCutIn' in patch) {
    if (typeof patch.allowCutIn === 'boolean') next.allowCutIn = patch.allowCutIn
    nextSources.allowCutIn = sourceFor(patch.allowCutIn)
  }
  if ('priorityTier' in patch) {
    if (patch.priorityTier !== null) next.priorityTier = patch.priorityTier ?? next.priorityTier
    nextSources.priorityTier = sourceFor(patch.priorityTier)
  }
  if ('fastModeRewriteMode' in patch) {
    if (patch.fastModeRewriteMode !== null) next.fastModeRewriteMode = patch.fastModeRewriteMode ?? next.fastModeRewriteMode
    nextSources.fastModeRewriteMode = sourceFor(patch.fastModeRewriteMode)
  }
  if ('imageToolRewriteMode' in patch) {
    if (patch.imageToolRewriteMode !== null) next.imageToolRewriteMode = patch.imageToolRewriteMode ?? next.imageToolRewriteMode
    nextSources.imageToolRewriteMode = sourceFor(patch.imageToolRewriteMode)
  }
  if ('concurrencyLimit' in patch) {
    if (patch.concurrencyLimit !== null) next.concurrencyLimit = patch.concurrencyLimit ?? next.concurrencyLimit
    nextSources.concurrencyLimit = sourceFor(patch.concurrencyLimit)
  }
  if ('upstream429RetryEnabled' in patch || 'upstream429MaxRetries' in patch) {
    const hasEnabled = Object.prototype.hasOwnProperty.call(patch, 'upstream429RetryEnabled')
    const hasRetries = Object.prototype.hasOwnProperty.call(patch, 'upstream429MaxRetries')
    const enabledValue = patch.upstream429RetryEnabled
    const retryValue = patch.upstream429MaxRetries
    if (enabledValue === null || retryValue === null) {
      next.upstream429RetryEnabled = false
      next.upstream429MaxRetries = 0
      nextSources.upstream429Retry = 'root'
    } else {
      if (enabledValue !== undefined) {
        next.upstream429RetryEnabled = enabledValue
      }
      if (retryValue !== undefined) {
        next.upstream429MaxRetries = retryValue
      }
      if (hasEnabled || hasRetries) {
        nextSources.upstream429Retry = 'account'
      }
    }
  }
  if ('availableModels' in patch) {
    if (patch.availableModels !== null) next.availableModels = patch.availableModels ?? next.availableModels
    nextSources.availableModels = sourceFor(patch.availableModels)
  }
  if ('timeouts' in patch && patch.timeouts) {
    const nextTimeoutSources = {
      responsesFirstByteTimeoutSecs: next.timeoutFieldSources?.responsesFirstByteTimeoutSecs ?? 'root',
      compactFirstByteTimeoutSecs: next.timeoutFieldSources?.compactFirstByteTimeoutSecs ?? 'root',
      responsesStreamTimeoutSecs: next.timeoutFieldSources?.responsesStreamTimeoutSecs ?? 'root',
      compactStreamTimeoutSecs: next.timeoutFieldSources?.compactStreamTimeoutSecs ?? 'root',
    }
    const nextTimeoutValues = {
      responsesFirstByteTimeoutSecs: next.timeouts?.responsesFirstByteTimeoutSecs ?? relaxedRule.timeouts!.responsesFirstByteTimeoutSecs,
      compactFirstByteTimeoutSecs: next.timeouts?.compactFirstByteTimeoutSecs ?? relaxedRule.timeouts!.compactFirstByteTimeoutSecs,
      responsesStreamTimeoutSecs: next.timeouts?.responsesStreamTimeoutSecs ?? relaxedRule.timeouts!.responsesStreamTimeoutSecs,
      compactStreamTimeoutSecs: next.timeouts?.compactStreamTimeoutSecs ?? relaxedRule.timeouts!.compactStreamTimeoutSecs,
    }
    for (const [key, value] of Object.entries(patch.timeouts)) {
      const timeoutKey = key as keyof typeof nextTimeoutValues
      if (value === null) {
        nextTimeoutValues[timeoutKey] = relaxedRule.timeouts![timeoutKey]
        nextTimeoutSources[timeoutKey] = 'root'
      } else if (typeof value === 'number') {
        nextTimeoutValues[timeoutKey] = value
        nextTimeoutSources[timeoutKey] = 'account'
      }
    }
    next.timeouts = nextTimeoutValues
    next.timeoutFieldSources = nextTimeoutSources
  }
  return next
}

function EditableRoutingRuleDemo({
  initialRule,
  busyField,
  errorByField,
}: {
  initialRule: EffectiveRoutingRule
  busyField?: EditablePolicyConfig['busyField']
  errorByField?: EditablePolicyConfig['errorByField']
}) {
  const [rule, setRule] = useState(initialRule)
  return (
    <EffectiveRoutingRuleCard
      rule={rule}
      labels={labels}
      editablePolicy={{
        busyField,
        errorByField,
        availableModelOptions: editableOptions,
        onChange: (_field, payload) => setRule((current) => applyPatchToRule(current, payload)),
      }}
    />
  )
}

export const EditableInherited: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={relaxedRule} />,
  play: async ({ canvasElement }) => {
    const timeoutButton = canvasElement.querySelector<HTMLButtonElement>(
      'button[aria-label="Edit account override: Standard response first byte timeout"]',
    )
    if (!timeoutButton) {
      throw new Error('missing inherited timeout edit button')
    }

    await userEvent.click(timeoutButton)

    expect(
      canvasElement.querySelector<HTMLInputElement>(
        'input[name="responsesFirstByteTimeoutSecs"]',
      ),
    ).not.toBeNull()
  },
}

export const EditableAccountOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
  play: async ({ canvasElement }) => {
    const rows = Array.from(
      canvasElement.querySelectorAll('div.border-b.border-base-300\\/60'),
    )

    function assertExpandedRowAligned(labelText: string, valueText: string) {
      const row = rows.find((candidate) => {
        const text = candidate.textContent || ''
        return text.includes(labelText) && text.includes(valueText) && text.includes('Account')
      })
      if (!row) {
        throw new Error(`missing expanded row for ${labelText}`)
      }

      const expandedGrid = row.querySelector('.border-t .grid')
      if (!(expandedGrid instanceof HTMLElement)) {
        throw new Error(`missing expanded grid for ${labelText}`)
      }

      const label = expandedGrid.children.item(0)
      const editor = expandedGrid.children.item(1)
      if (!(label instanceof HTMLElement) || !(editor instanceof HTMLElement)) {
        throw new Error(`missing expanded content for ${labelText}`)
      }

      const textNode = Array.from(label.childNodes).find(
        (node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim(),
      )
      if (!textNode) {
        throw new Error(`missing label text node for ${labelText}`)
      }

      const range = document.createRange()
      range.selectNodeContents(textNode)
      const textRect = range.getBoundingClientRect()
      const editorRect = editor.getBoundingClientRect()
      const textCenterY = textRect.top + textRect.height / 2
      const editorCenterY = editorRect.top + editorRect.height / 2

      expect(Math.abs(textCenterY - editorCenterY)).toBeLessThanOrEqual(6)
    }

    assertExpandedRowAligned('FAST mode', 'Force remove')
    assertExpandedRowAligned('Upstream 429 retry', '4')
  },
}

export const EditableMultipleAccountOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={multipleAccountOverridesRule} />,
}

export const EditableTimeoutOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={multipleAccountOverridesRule} />,
  parameters: {
    docs: {
      description: {
        story:
          'Account effective rule with mixed timeout inheritance: account overrides on first-byte and stream fields, inherited group/default values on the remaining fields.',
      },
    },
  },
}

export const EditableSavingAndError: Story = {
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={strictRule}
      busyField="priorityTier"
      errorByField={{ allowCutIn: 'Save failed. Check the account policy and retry.' }}
    />
  ),
}

export const EditableDenyAllModels: Story = {
  render: () => (
    <EditableRoutingRuleDemo
      initialRule={{
        ...strictRule,
        availableModels: [],
        fieldSources: {
          blockNewConversations: strictRule.fieldSources?.blockNewConversations ?? 'root',
          allowCutOut: strictRule.fieldSources?.allowCutOut ?? 'root',
          allowCutIn: strictRule.fieldSources?.allowCutIn ?? 'root',
          priorityTier: strictRule.fieldSources?.priorityTier ?? 'root',
          fastModeRewriteMode: strictRule.fieldSources?.fastModeRewriteMode ?? 'root',
          imageToolRewriteMode: strictRule.fieldSources?.imageToolRewriteMode ?? 'root',
          concurrencyLimit: strictRule.fieldSources?.concurrencyLimit ?? 'root',
          upstream429Retry: strictRule.fieldSources?.upstream429Retry ?? 'root',
          ...strictRule.fieldSources,
          availableModels: 'account',
        },
      }}
    />
  ),
}
