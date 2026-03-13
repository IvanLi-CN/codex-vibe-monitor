import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import { InvocationRecordsTable } from './InvocationRecordsTable'
import { STORYBOOK_INVOCATION_RECORDS } from './invocationRecordsStoryFixtures'
import { STORYBOOK_COLOR_CONTRAST_TODO } from '../storybook/a11y'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div data-theme="light" className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-[1440px]">{children}</div>
    </div>
  )
}

const meta = {
  title: 'Records/InvocationRecordsTable',
  component: InvocationRecordsTable,
  parameters: {
    ...STORYBOOK_COLOR_CONTRAST_TODO,
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationRecordsTable>

export default meta

type Story = StoryObj<typeof meta>

export const TokenFocus: Story = {
  args: {
    focus: 'token',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const NetworkFocus: Story = {
  args: {
    focus: 'network',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const ExceptionFocus: Story = {
  args: {
    focus: 'exception',
    records: STORYBOOK_INVOCATION_RECORDS,
    isLoading: false,
    error: null,
  },
}

export const Loading: Story = {
  args: {
    focus: 'token',
    records: [],
    isLoading: true,
    error: null,
  },
}

export const Empty: Story = {
  args: {
    focus: 'token',
    records: [],
    isLoading: false,
    error: null,
  },
}
