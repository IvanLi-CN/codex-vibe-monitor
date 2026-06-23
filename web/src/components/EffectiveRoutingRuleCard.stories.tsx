import type { Meta, StoryObj } from '@storybook/react-vite'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

const labels = {
  title: 'Effective routing rule',
  description: 'Merged routing constraints applied to the selected upstream account.',
  noTags: 'No tags linked',
  blockNewConversations: 'Block new conversations',
  allowNewConversations: 'New conversations are not blocked',
  allowCutOut: 'Cut-out not blocked',
  denyCutOut: 'Cut-out blocked',
  allowCutIn: 'Cut-in not blocked',
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
  fieldBlockNewConversations: 'Block new conversations',
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
