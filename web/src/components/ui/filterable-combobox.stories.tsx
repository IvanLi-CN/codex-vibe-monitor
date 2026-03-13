import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { FilterableCombobox } from './filterable-combobox'

const noop = () => {}

const inputClassName =
  'h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-md">{children}</div>
    </div>
  )
}

function ControlledCombobox({
  label,
  options,
  placeholder,
  emptyText,
  disabled,
}: {
  label: string
  options: string[]
  placeholder?: string
  emptyText?: string
  disabled?: boolean
}) {
  const [value, setValue] = useState('')
  return (
    <StorySurface>
      <div className="space-y-4">
        <FilterableCombobox
          label={label}
          name="storybook-filterable-combobox"
          value={value}
          onValueChange={setValue}
          options={options}
          placeholder={placeholder}
          emptyText={emptyText}
          disabled={disabled}
          inputClassName={inputClassName}
        />
        <div className="rounded-xl border border-base-300/70 bg-base-100/45 px-4 py-3 text-sm text-base-content/70">
          Current value: <span className="font-mono text-base-content">{value || '—'}</span>
        </div>
      </div>
    </StorySurface>
  )
}

const meta = {
  title: 'UI/FilterableCombobox',
  component: FilterableCombobox,
} satisfies Meta<typeof FilterableCombobox>

export default meta

type Story = StoryObj<typeof meta>

export const Basic: Story = {
  args: {
    label: 'Combobox',
    value: '',
    onValueChange: noop,
    options: [],
  },
  render: () => (
    <ControlledCombobox
      label="Model"
      placeholder="Any"
      emptyText="No matches"
      options={[
        'gpt-4o',
        'gpt-4o-mini',
        'o1',
        'o3-mini',
        'claude-3.7-sonnet',
        'qwen2.5-72b',
      ]}
    />
  ),
}

export const ManyOptions: Story = {
  args: {
    label: 'Combobox',
    value: '',
    onValueChange: noop,
    options: [],
  },
  render: () => (
    <ControlledCombobox
      label="Endpoint"
      placeholder="Any"
      emptyText="No matches"
      options={[
        '/v1/chat/completions',
        '/v1/responses',
        '/v1/embeddings',
        '/v1/audio/transcriptions',
        '/v1/images/generations',
        '/v1/moderations',
        '/v1/batches',
        '/v1/files',
        '/v1/assistants',
        '/v1/vector_stores',
        '/v1/fine_tuning/jobs',
        '/v1/models',
      ]}
    />
  ),
}

export const Disabled: Story = {
  args: {
    label: 'Combobox',
    value: '',
    onValueChange: noop,
    options: [],
  },
  render: () => (
    <ControlledCombobox
      label="Proxy"
      placeholder="Any"
      emptyText="No matches"
      disabled
      options={['proxy-a', 'proxy-b', 'proxy-c']}
    />
  ),
}
