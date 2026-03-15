import type { ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Stories } from '@storybook/addon-docs/blocks'
import { Input } from './input'
import { FormFieldFeedback } from './form-field-feedback'

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="bg-base-200 px-4 py-4 text-base-content">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 rounded-2xl border border-base-300/60 bg-base-100/70 p-5 shadow-sm">
        {children}
      </div>
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
      page: () => <Stories title="" includePrimary={true} />,
      canvas: {
        sourceState: 'hidden',
      },
      controls: {
        hideNoControlsWarning: true,
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
  args: {
    label: '上游地址',
    message: null,
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
