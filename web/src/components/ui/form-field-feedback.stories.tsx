import type { ReactNode } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { Stories } from '@storybook/addon-docs/blocks'
import type { BubbleVariant } from './bubble'
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
  variant = 'error',
  placeholder,
}: {
  label: string
  message?: string | null
  variant?: BubbleVariant
  placeholder?: string
}) {
  const isError = variant === 'error'
  const inputToneClass = variant === 'success'
    ? 'border-success/70 focus-visible:ring-success'
    : variant === 'warning'
      ? 'border-warning/70 focus-visible:ring-warning'
      : isError
        ? 'border-error/70 focus-visible:ring-error'
        : ''

  return (
    <label className="field">
      <FormFieldFeedback
        label={label}
        message={message}
        variant={variant}
        messageClassName="md:max-w-[min(30rem,calc(100%-9rem))]"
      />
      <Input
        value={placeholder ?? ''}
        placeholder={placeholder}
        readOnly
        aria-invalid={isError ? 'true' : 'false'}
        className={message ? inputToneClass : ''}
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
    variant: {
      control: 'radio',
      options: ['error', 'warning', 'success', 'info', 'neutral'],
    },
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
    variant: 'error',
  },
  render: (args) => (
    <FieldHarness
      label={String(args.label)}
      message={typeof args.message === 'string' ? args.message : undefined}
      variant={args.variant}
      placeholder="proxy.example.com/gateway"
    />
  ),
}

export const QuietField: Story = {
  args: {
    label: '上游地址',
    message: null,
    variant: 'error',
  },
  render: (args) => (
    <FieldHarness
      label={String(args.label)}
      message={typeof args.message === 'string' ? args.message : undefined}
      variant={args.variant}
      placeholder="https://proxy.example.com/gateway"
    />
  ),
}

export const SuccessField: Story = {
  args: {
    label: '回调地址',
    message: '地址格式正确，授权完成后即可继续下一步。',
    variant: 'success',
  },
  render: (args) => (
    <FieldHarness
      label={String(args.label)}
      message={typeof args.message === 'string' ? args.message : undefined}
      variant={args.variant}
      placeholder="https://proxy.example.com/oauth/callback"
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
