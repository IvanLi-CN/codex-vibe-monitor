import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import { InvocationRecordsSummaryCards } from './InvocationRecordsSummaryCards'
import { createStoryInvocationRecordsSummary } from './invocationRecordsStoryFixtures'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1440px]">{children}</div>
    </div>
  )
}

const meta = {
  title: 'Records/InvocationRecordsSummaryCards',
  component: InvocationRecordsSummaryCards,
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationRecordsSummaryCards>

export default meta

type Story = StoryObj<typeof meta>

export const TokenFocus: Story = {
  args: {
    focus: 'token',
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
}

export const NetworkFocus: Story = {
  args: {
    focus: 'network',
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
}

export const ExceptionFocus: Story = {
  args: {
    focus: 'exception',
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    focus: 'token',
    summary: null,
    isLoading: true,
    error: null,
  },
}

export const LoadError: Story = {
  args: {
    focus: 'token',
    summary: null,
    isLoading: false,
    error: 'Request failed: 500 database is busy',
  },
}
