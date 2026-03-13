import { useState } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { UpstreamAccountGroupCombobox } from './UpstreamAccountGroupCombobox'
import { STORYBOOK_COLOR_CONTRAST_TODO } from '../storybook/a11y'

function ComboboxHarness({
  value: initialValue,
  suggestions,
  placeholder,
}: {
  value: string
  suggestions: string[]
  placeholder?: string
}) {
  const [value, setValue] = useState(initialValue)
  return (
    <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto max-w-md">
        <UpstreamAccountGroupCombobox
          value={value}
          suggestions={suggestions}
          placeholder={placeholder}
          onValueChange={setValue}
        />
      </div>
    </div>
  )
}

const meta = {
  title: 'Account Pool/Components/Upstream Account Group Combobox',
  component: UpstreamAccountGroupCombobox,
  tags: ['autodocs'],
  parameters: {
    ...STORYBOOK_COLOR_CONTRAST_TODO,
    layout: 'fullscreen',
  },
  args: {
    value: '',
    onValueChange: () => undefined,
    suggestions: ['production', 'staging', 'shared-services'],
    placeholder: 'Select or type a group',
  },
  render: (args) => (
    <ComboboxHarness
      value={args.value}
      suggestions={args.suggestions}
      placeholder={args.placeholder}
    />
  ),
} satisfies Meta<typeof UpstreamAccountGroupCombobox>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const WithExistingValue: Story = {
  args: {
    value: 'production',
  },
}
