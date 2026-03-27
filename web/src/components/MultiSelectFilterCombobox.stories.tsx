import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { MultiSelectFilterCombobox } from './MultiSelectFilterCombobox'

const options = [
  { value: 'working', label: 'Working' },
  { value: 'idle', label: 'Idle' },
  { value: 'rate_limited', label: 'Rate limited' },
]

function StoryHarness() {
  const [value, setValue] = useState<string[]>(['working', 'rate_limited'])

  return (
    <div className="w-full max-w-sm p-6">
      <MultiSelectFilterCombobox
        options={options}
        value={value}
        placeholder="All work statuses"
        searchPlaceholder="Search work statuses..."
        emptyLabel="No matching work statuses."
        clearLabel="Clear work status filters"
        ariaLabel="Work status"
        size="filter"
        onValueChange={setValue}
      />
    </div>
  )
}

const meta = {
  title: 'Account Pool/Multi Select Filter Combobox',
  component: StoryHarness,
  tags: ['autodocs'],
} satisfies Meta<typeof StoryHarness>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}
