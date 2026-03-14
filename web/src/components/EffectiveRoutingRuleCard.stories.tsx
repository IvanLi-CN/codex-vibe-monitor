import type { Meta, StoryObj } from '@storybook/react-vite'
import type { EffectiveRoutingRule } from '../lib/api'
import { EffectiveRoutingRuleCard } from './EffectiveRoutingRuleCard'

const labels = {
  title: 'Effective routing rule',
  description: 'Merged routing constraints applied to the selected upstream account.',
  noTags: 'No tags linked',
  guardEnabled: 'Conversation guard on',
  guardDisabled: 'Conversation guard off',
  allowCutOut: 'Cut-out allowed',
  denyCutOut: 'Cut-out blocked',
  allowCutIn: 'Cut-in allowed',
  denyCutIn: 'Cut-in blocked',
  sourceTags: 'Source tags',
  guardRule: (hours: number, count: number) => `${hours}h / ${count} conversations`,
  allGuardsApply: 'All guard rules apply together',
}

const relaxedRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const strictRule: EffectiveRoutingRule = {
  guardEnabled: true,
  lookbackHours: 6,
  maxConversations: 4,
  allowCutOut: false,
  allowCutIn: false,
  sourceTagIds: [1, 2],
  sourceTagNames: ['vip-routing', 'handoff-blocked'],
  guardRules: [
    { tagId: 1, tagName: 'vip-routing', lookbackHours: 6, maxConversations: 4 },
    { tagId: 2, tagName: 'handoff-blocked', lookbackHours: 2, maxConversations: 2 },
  ],
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
