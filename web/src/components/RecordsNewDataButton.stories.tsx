import type { Meta, StoryObj } from '@storybook/react-vite'
import { I18nProvider } from '../i18n'
import {
  RecordsNewDataButton,
  type RecordsNewDataButtonProps,
} from './RecordsNewDataButton'

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto flex w-full max-w-xl items-center justify-center">{children}</div>
    </div>
  )
}

function StoryHarness(props: Partial<RecordsNewDataButtonProps>) {
  return (
    <I18nProvider>
      <RecordsNewDataButton
        count={17}
        isLoading={false}
        onRefresh={() => {}}
        {...props}
      />
    </I18nProvider>
  )
}

const meta = {
  title: 'Records/RecordsNewDataButton',
  component: RecordsNewDataButton,
  args: {
    count: 17,
    onRefresh: () => {},
  },
  decorators: [
    (Story) => (
      <StorySurface>
        <Story />
      </StorySurface>
    ),
  ],
} satisfies Meta<typeof RecordsNewDataButton>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {
  render: (args) => <StoryHarness {...args} />,
}

export const Interactive: Story = {
  args: {
    stateOverride: 'interactive',
  },
  render: (args) => <StoryHarness {...args} />,
}

export const Loading: Story = {
  args: {
    isLoading: true,
  },
  render: (args) => <StoryHarness {...args} />,
}

export const StateGallery: Story = {
  render: () => (
    <I18nProvider>
      <div className="flex flex-wrap items-center gap-4">
        <RecordsNewDataButton count={17} onRefresh={() => {}} />
        <RecordsNewDataButton count={17} onRefresh={() => {}} stateOverride="interactive" />
        <RecordsNewDataButton count={17} onRefresh={() => {}} isLoading />
      </div>
    </I18nProvider>
  ),
}
