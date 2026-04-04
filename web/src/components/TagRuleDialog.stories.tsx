import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import type { TagSummary } from '../lib/api'
import { TagRuleDialog } from './TagRuleDialog'

type DialogHarnessProps = {
  mode: 'create' | 'edit'
  tag?: TagSummary | null
  draftName?: string
}

function DialogHarness({ tag = null, draftName, mode }: DialogHarnessProps) {
  const [open, setOpen] = useState(true)

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Routing Rules
          </p>
          <h1 className="text-2xl font-semibold">Tag routing rule dialog</h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            Stable preview for the tag concurrency slider and routing toggles.
          </p>
        </div>
        <TagRuleDialog
          open={open}
          mode={mode}
          tag={tag}
          draftName={draftName}
          onClose={() => setOpen(false)}
          onSubmit={() => undefined}
          labels={{
            createTitle: 'Create tag',
            editTitle: 'Edit tag',
            description: 'Adjust the routing rules that accounts under this tag must follow.',
            name: 'Tag name',
            namePlaceholder: 'vip, night-shift, warm-standby',
            guardEnabled: 'Conversation guard',
            lookbackHours: 'Lookback hours',
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
            concurrencyHint: 'Use 1-30 to cap fresh assignments. The last slider step means unlimited.',
            currentValue: 'Current',
            unlimited: 'Unlimited',
            cancel: 'Cancel',
            save: 'Save tag',
            create: 'Create tag',
            validation: 'When the guard is enabled, both guard values must be positive integers.',
          }}
        />
      </div>
    </div>
  )
}

const finiteTag: TagSummary = {
  id: 7,
  name: 'priority-lane',
  routingRule: {
    guardEnabled: true,
    lookbackHours: 6,
    maxConversations: 10,
    allowCutOut: true,
    allowCutIn: false,
    priorityTier: 'primary',
    fastModeRewriteMode: 'force_add',
    concurrencyLimit: 6,
  },
  accountCount: 4,
  groupCount: 2,
  updatedAt: '2026-03-29T12:00:00.000Z',
}

const unlimitedTag: TagSummary = {
  id: 8,
  name: 'overflow',
  routingRule: {
    guardEnabled: false,
    lookbackHours: null,
    maxConversations: null,
    allowCutOut: false,
    allowCutIn: true,
    priorityTier: 'fallback',
    fastModeRewriteMode: 'force_remove',
    concurrencyLimit: 0,
  },
  accountCount: 2,
  groupCount: 1,
  updatedAt: '2026-03-29T18:30:00.000Z',
}

const meta = {
  title: 'Account Pool/Components/Tag Rule Dialog',
  component: DialogHarness,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  render: (args) => <DialogHarness {...args} />,
  args: {
    mode: 'edit',
    tag: finiteTag,
    draftName: '',
  },
} satisfies Meta<typeof DialogHarness>

export default meta

type Story = StoryObj<typeof meta>

export const FiniteLimit: Story = {}

export const UnlimitedLimit: Story = {
  args: {
    mode: 'edit',
    tag: unlimitedTag,
  },
}

export const CreateDialog: Story = {
  args: {
    mode: 'create',
    tag: null,
    draftName: 'new-lane',
  },
}

export const ForceRemoveMode: Story = {
  args: {
    mode: 'edit',
    tag: unlimitedTag,
  },
}
