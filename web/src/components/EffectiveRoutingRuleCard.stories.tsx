import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
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
  upstream429Retry: '429 retry enabled',
  upstream429RetryOff: '429 retry off',
  availableModelsInherited: 'Inherited / unrestricted',
  availableModelsNoneAllowed: 'No models allowed',
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
  upstream429RetryCountOnce: 'Retry once',
  upstream429RetryCountMany: (count: number) => `Retry ${count} times`,
  availableModelsAddCustom: 'Add model',
  availableModelsCustomLabel: (value: string) => `Add ${value}`,
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
  if ('blockNewConversations' in patch) {
    if (typeof patch.blockNewConversations === 'boolean') next.blockNewConversations = patch.blockNewConversations
    nextSources.blockNewConversations = sourceFor(patch.blockNewConversations)
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
    if (patch.upstream429RetryEnabled !== null && patch.upstream429RetryEnabled !== undefined) {
      next.upstream429RetryEnabled = patch.upstream429RetryEnabled
    }
    if (patch.upstream429MaxRetries !== null && patch.upstream429MaxRetries !== undefined) {
      next.upstream429MaxRetries = patch.upstream429MaxRetries
    }
    nextSources.upstream429Retry = sourceFor(patch.upstream429RetryEnabled ?? patch.upstream429MaxRetries)
  }
  if ('availableModels' in patch) {
    if (patch.availableModels !== null) next.availableModels = patch.availableModels ?? next.availableModels
    nextSources.availableModels = sourceFor(patch.availableModels)
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
}

export const EditableAccountOverrides: Story = {
  render: () => <EditableRoutingRuleDemo initialRule={strictRule} />,
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
