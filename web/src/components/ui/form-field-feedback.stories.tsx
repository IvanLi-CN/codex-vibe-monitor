import type { ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Input } from './input'
import { FormFieldFeedback } from './form-field-feedback'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">{children}</div>
    </div>
  )
}

function FieldHarness({
  label,
  message,
  placeholder,
}: {
  label: string
  message?: string | null
  placeholder?: string
}) {
  return (
    <label className="field">
      <FormFieldFeedback
        label={label}
        message={message}
        messageClassName="md:max-w-[min(30rem,calc(100%-9rem))]"
      />
      <Input
        value={placeholder ?? ''}
        placeholder={placeholder}
        readOnly
        aria-invalid={message ? 'true' : 'false'}
        className={message ? 'border-error/70 focus-visible:ring-error' : ''}
      />
    </label>
  )
}

const meta = {
  title: 'UI/FormFieldFeedback',
  component: FormFieldFeedback,
  tags: ['autodocs'],
  parameters: {
    layout: 'padded',
    docs: {
      description: {
        component:
          'Shared label-row feedback for form fields. It keeps the validation bubble in the reserved space beside the label instead of overlaying the next field.',
      },
    },
  },
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
  argTypes: {
    label: {
      control: 'text',
    },
    message: {
      control: 'text',
    },
    className: {
      control: false,
    },
    labelClassName: {
      control: false,
    },
    messageClassName: {
      control: false,
    },
  },
} satisfies Meta<typeof FormFieldFeedback>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  args: {
    label: '上游地址',
    message: '请填写 http(s) 的绝对 URL，例如 https://proxy.example.com/gateway',
  },
  render: (args) => (
    <FieldHarness
      label={String(args.label)}
      message={typeof args.message === 'string' ? args.message : undefined}
      placeholder="proxy.example.com/gateway"
    />
  ),
}

export const QuietField: Story = {
  args: {
    label: '上游地址',
    message: null,
  },
  render: (args) => (
    <FieldHarness
      label={String(args.label)}
      message={typeof args.message === 'string' ? args.message : undefined}
      placeholder="https://proxy.example.com/gateway"
    />
  ),
}

export const DenseTwoColumnLayout: Story = {
  parameters: {
    docs: {
      description: {
        story:
          'Matches the account-pool form density: the message stays in the label row, leaving the next row untouched.',
      },
    },
  },
  render: () => (
    <div className="grid gap-4 md:grid-cols-2">
      <FieldHarness label="上游地址" message="请填写 http(s) 的绝对 URL，例如 https://proxy.example.com/gateway" placeholder="proxy.example.com/gateway" />
      <FieldHarness label="限额单位" message={null} placeholder="tokens" />
      <FieldHarness label="5 小时本地限额" message={null} placeholder="" />
      <FieldHarness label="7 天本地限额" message={null} placeholder="" />
    </div>
  ),
}
