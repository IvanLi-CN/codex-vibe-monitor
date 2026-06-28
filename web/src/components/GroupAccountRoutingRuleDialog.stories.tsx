import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import type { GroupAccountRoutingRule } from '../lib/api'
import { GroupAccountRoutingRuleDialog } from './GroupAccountRoutingRuleDialog'

type DialogHarnessProps = {
  rule?: GroupAccountRoutingRule | null
  title?: string
  description?: string
  submitLabel?: string
}

function DialogHarness({
  rule = null,
  title = 'Group routing policy',
  description = 'Routing policy inherited by accounts in this group.',
  submitLabel = 'Apply group policy',
}: DialogHarnessProps) {
  const [open, setOpen] = useState(true)
  const availableModelOptions = [
    'gpt-5.5',
    'gpt-5.5-pro',
    'gpt-5.4',
    'gpt-5.4-pro',
    'gpt-5.3-codex',
  ]

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Routing Rules
          </p>
          <h1 className="text-2xl font-semibold">Group/account routing rule dialog</h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            Stable preview for the policy editor shared by group and account routing.
          </p>
        </div>
        <GroupAccountRoutingRuleDialog
          open={open}
          rule={rule}
          title={title}
          description={description}
          submitLabel={submitLabel}
          onClose={() => setOpen(false)}
          onSubmit={() => undefined}
          availableModelOptions={availableModelOptions}
          labels={{
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
            availableModelsHint:
              'Leave empty to inherit. Automatic and sticky routing only consider matching accounts.',
            availableModelsSearchPlaceholder: 'Search models',
            availableModelsEmpty: 'No matching models',
            availableModelsAll: 'Inherited / unrestricted',
            availableModelsCustomLabel: (value) => value,
            availableModelsAddCustom: 'Add custom model id',
            availableModelsInherited: 'Clear and inherit',
            availableModelsRemove: 'Remove model',
            upstream429Retry: 'Upstream 429 retry',
            upstream429RetryHint: 'Retry the same upstream account before cooldown and failover.',
            upstream429RetryToggle: 'Retry after upstream 429',
            upstream429RetryCount: 'Retry count',
            upstream429RetryCountOnce: '1 retry',
            upstream429RetryCountMany: (count) => `${count} retries`,
            cancel: 'Cancel',
            validation: 'Review the routing policy before saving.',
          }}
        />
      </div>
    </div>
  )
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

const forceRemoveRule: GroupAccountRoutingRule = {
  ...defaultRule,
  imageToolRewriteMode: 'force_remove',
}

const meta = {
  title: 'Account Pool/Components/Group Account Routing Rule Dialog',
  component: DialogHarness,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  render: (args) => <DialogHarness {...args} />,
  args: {
    rule: defaultRule,
  },
} satisfies Meta<typeof DialogHarness>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const ForceRemoveImageTool: Story = {
  args: {
    rule: forceRemoveRule,
  },
}
