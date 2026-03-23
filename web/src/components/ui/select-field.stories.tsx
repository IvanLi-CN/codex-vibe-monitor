import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { SelectField } from './select-field'

const noop = () => {}

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-md">{children}</div>
    </div>
  )
}

function ControlledSelectField({
  label,
  initialValue,
  placeholder,
  size,
  disabled,
}: {
  label: string
  initialValue: string
  placeholder?: string
  size?: 'default' | 'sm' | 'filter'
  disabled?: boolean
}) {
  const [value, setValue] = useState(initialValue)

  return (
    <StorySurface>
      <div className="space-y-4">
        <SelectField
          label={label}
          name="storybook-select-field"
          value={value}
          onValueChange={setValue}
          placeholder={placeholder}
          size={size}
          disabled={disabled}
          options={[
            { value: '', label: 'All windows' },
            { value: '20', label: '20 conversations' },
            { value: '50', label: '50 conversations' },
            { value: '100', label: '100 conversations' },
          ]}
        />
        <div className="rounded-xl border border-base-300/70 bg-base-100/45 px-4 py-3 text-sm text-base-content/70">
          Current value: <span className="font-mono text-base-content">{value || '—'}</span>
        </div>
      </div>
    </StorySurface>
  )
}

const meta = {
  title: 'UI/SelectField',
  component: SelectField,
  args: {
    options: [],
    value: '',
    onValueChange: noop,
  },
} satisfies Meta<typeof SelectField>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: () => (
    <ControlledSelectField
      label="Conversation filter"
      initialValue="50"
    />
  ),
}

export const Placeholder: Story = {
  render: () => (
    <StorySurface>
      <SelectField
        label="Aggregation"
        value=""
        onValueChange={() => undefined}
        placeholder="Select a bucket"
        options={[
          { value: '15m', label: 'Every 15 minutes' },
          { value: '1h', label: 'Every hour' },
          { value: '1d', label: 'Every 24 hours' },
        ]}
      />
    </StorySurface>
  ),
}

export const Small: Story = {
  render: () => (
    <ControlledSelectField
      label="Page size"
      initialValue="20"
      size="sm"
    />
  ),
}

export const Disabled: Story = {
  render: () => (
    <ControlledSelectField
      label="Proxy policy"
      initialValue="100"
      disabled
    />
  ),
}

export const Filter: Story = {
  render: () => (
    <ControlledSelectField
      label="Account status"
      initialValue=""
      placeholder="All account states"
      size="filter"
    />
  ),
}
