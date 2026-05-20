import type { Meta, StoryObj } from '@storybook/react-vite'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

const labels = {
  title: 'Effective routing rule',
  description: 'Merged routing constraints applied to the selected upstream account.',
  noTags: 'No tags linked',
  guardEnabled: 'Block new conversations',
  guardDisabled: 'New conversations are not blocked',
  allowCutOut: 'Cut-out not blocked',
  denyCutOut: 'Cut-out blocked',
  allowCutIn: 'Cut-in not blocked',
  denyCutIn: 'Cut-in blocked',
  sourceTags: 'Source tags',
  guardRule: (hours: number, count: number) => `${hours}h / ${count} conversations`,
  allGuardsApply: 'All guard rules apply together',
  priorityPrimary: 'Primary',
  priorityNormal: 'Normal',
  priorityFallback: 'Fallback only',
  fastModeKeepOriginal: 'Keep original',
  fastModeFillMissing: 'Fill when missing',
  fastModeForceAdd: 'Force add',
  fastModeForceRemove: 'Force remove',
  upstream429Retry: '429 retry enabled',
  upstream429RetryOff: '429 retry off',
  concurrencyLimit: (count: number) => `Concurrency ${count}`,
  concurrencyUnlimited: 'Concurrency unlimited',
  sourceBreakdownTitle: 'Field source breakdown',
  fieldGuard: 'Conversation guard',
  fieldAllowCutOut: 'Cut out',
  fieldAllowCutIn: 'Cut in',
  fieldPriority: 'Priority',
  fieldFastMode: 'FAST mode',
  fieldConcurrency: 'Concurrency',
  fieldUpstream429: 'Upstream 429 retry',
  sourceRoot: 'Root default',
  sourceGroup: 'Group',
  sourceTag: 'Tag',
  sourceAccount: 'Account',
}

const relaxedRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  priorityTier: 'normal',
  fastModeRewriteMode: 'keep_original',
  concurrencyLimit: 0,
  upstream429RetryEnabled: false,
  upstream429MaxRetries: 0,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
  fieldSources: {
    guard: 'root',
    allowCutOut: 'root',
    allowCutIn: 'root',
    priorityTier: 'root',
    fastModeRewriteMode: 'root',
    concurrencyLimit: 'root',
    upstream429Retry: 'root',
  },
}

const strictRule: EffectiveRoutingRule = {
  guardEnabled: true,
  lookbackHours: 6,
  maxConversations: 4,
  allowCutOut: false,
  allowCutIn: false,
  priorityTier: 'fallback',
  fastModeRewriteMode: 'force_remove',
  concurrencyLimit: 2,
  upstream429RetryEnabled: true,
  upstream429MaxRetries: 4,
  sourceTagIds: [1, 2],
  sourceTagNames: ['vip-routing', 'handoff-blocked'],
  guardRules: [
    { tagId: 1, tagName: 'vip-routing', lookbackHours: 6, maxConversations: 4 },
    { tagId: 2, tagName: 'handoff-blocked', lookbackHours: 2, maxConversations: 2 },
  ],
  fieldSources: {
    guard: 'group',
    allowCutOut: 'tag',
    allowCutIn: 'account',
    priorityTier: 'tag',
    fastModeRewriteMode: 'account',
    concurrencyLimit: 'tag',
    upstream429Retry: 'account',
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
