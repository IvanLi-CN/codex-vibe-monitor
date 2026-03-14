import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { UpstreamAccountGroupNoteDialog } from './UpstreamAccountGroupNoteDialog'

type DialogHarnessProps = {
  groupName: string
  note: string
  existing: boolean
  busy?: boolean
  error?: string | null
}

function DialogHarness({ note: initialNote, ...args }: DialogHarnessProps) {
  const [note, setNote] = useState(initialNote)

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Group Notes
          </p>
          <h1 className="text-2xl font-semibold">Upstream account group note dialog</h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            This story focuses on the shared note editor used across batch OAuth, single OAuth,
            API key creation, and account detail editing.
          </p>
        </div>
        <UpstreamAccountGroupNoteDialog
          open
          {...args}
          note={note}
          onNoteChange={setNote}
          onClose={() => undefined}
          onSave={() => undefined}
          title="Edit group note"
          existingDescription="This note is already shared by accounts inside the group and saves immediately."
          draftDescription="This draft stays on the page until the first account actually lands in the group."
          noteLabel="Group note"
          notePlaceholder="Capture what this group is for, ownership, and any operational caveats."
          cancelLabel="Cancel"
          saveLabel="Save group note"
          closeLabel="Close dialog"
          existingBadgeLabel="Persisted group"
          draftBadgeLabel="Draft group"
        />
      </div>
    </div>
  )
}

const meta = {
  title: 'Account Pool/Components/Upstream Account Group Note Dialog',
  component: UpstreamAccountGroupNoteDialog,
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
  },
} satisfies Meta<typeof UpstreamAccountGroupNoteDialog>

export default meta

type Story = StoryObj<typeof meta>

export const ExistingGroup: Story = {}

export const DraftGroup: Story = {
  args: {
    groupName: 'new-team',
    note: 'Draft note to keep onboarding context before the first account finishes authorization.',
    existing: false,
  },
}

export const SaveError: Story = {
  args: {
    error: 'Could not refresh the shared group list. The saved note stays visible so you can retry safely.',
  },
}
