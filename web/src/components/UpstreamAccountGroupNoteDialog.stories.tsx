import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ForwardProxyBindingNode } from '../lib/api'
import { UpstreamAccountGroupNoteDialog } from './UpstreamAccountGroupNoteDialog'

type DialogHarnessProps = {
  groupName: string
  note: string
  existing: boolean
  busy?: boolean
  error?: string | null
  boundProxyKeys?: string[]
  availableProxyNodes?: ForwardProxyBindingNode[]
}

const defaultForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: 'jp-edge-01',
    source: 'manual',
    displayName: 'JP Edge 01',
    penalized: false,
    selectable: true,
  },
  {
    key: 'sg-edge-02',
    source: 'subscription',
    displayName: 'SG Edge 02',
    penalized: false,
    selectable: true,
  },
  {
    key: 'drain-node',
    source: 'manual',
    displayName: 'Drain Node',
    penalized: true,
    selectable: false,
  },
]

function DialogHarness({
  note: initialNote,
  boundProxyKeys: initialBoundProxyKeys = [],
  availableProxyNodes = defaultForwardProxyNodes,
  ...args
}: DialogHarnessProps) {
  const [note, setNote] = useState(initialNote)
  const [boundProxyKeys, setBoundProxyKeys] = useState(initialBoundProxyKeys)

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Group Settings
          </p>
          <h1 className="text-2xl font-semibold">Upstream account group settings dialog</h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            This story focuses on the shared group note editor plus hard binding for forward proxy nodes.
          </p>
        </div>
        <UpstreamAccountGroupNoteDialog
          open
          {...args}
          note={note}
          boundProxyKeys={boundProxyKeys}
          availableProxyNodes={availableProxyNodes}
          onNoteChange={setNote}
          onBoundProxyKeysChange={setBoundProxyKeys}
          onClose={() => undefined}
          onSave={() => undefined}
          title="Edit group settings"
          existingDescription="This group already exists. Saving here updates the shared note and proxy bindings immediately."
          draftDescription="This group is not populated yet. Saving here creates its shared settings in advance."
          noteLabel="Group note"
          notePlaceholder="Capture what this group is for, ownership, and any operational caveats."
          cancelLabel="Cancel"
          saveLabel="Save group settings"
          closeLabel="Close dialog"
          existingBadgeLabel="Persisted group"
          draftBadgeLabel="Draft group"
          proxyBindingsLabel="Bound proxy nodes"
          proxyBindingsHint="Leave empty to keep automatic routing. Selected nodes are used as a hard-bound pool for this group."
          proxyBindingsAutomaticLabel="No nodes bound. This group uses automatic routing."
          proxyBindingsEmptyLabel="No proxy nodes available."
          proxyBindingsMissingLabel="Missing"
          proxyBindingsUnavailableLabel="Unavailable"
        />
      </div>
    </div>
  )
}

const meta = {
  title: 'Account Pool/Components/Upstream Account Group Settings Dialog',
  component: DialogHarness,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  render: (args) => <DialogHarness {...args} />,
  args: {
    groupName: 'production',
    note: 'Primary team group for premium traffic and shared routing policies.',
    existing: true,
    busy: false,
    error: null,
    boundProxyKeys: [],
    availableProxyNodes: defaultForwardProxyNodes,
  },
} satisfies Meta<typeof DialogHarness>

export default meta

type Story = StoryObj<typeof meta>

export const AutomaticRouting: Story = {}

export const HardBoundMultipleNodes: Story = {
  args: {
    boundProxyKeys: ['jp-edge-01', 'sg-edge-02'],
  },
}

export const MissingOrUnavailableBindings: Story = {
  args: {
    groupName: 'overflow',
    note: 'Legacy overflow group with one stale node reference.',
    boundProxyKeys: ['drain-node', 'missing-node-legacy'],
  },
}
