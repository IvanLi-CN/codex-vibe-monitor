import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import {
  AccountPoolStoryRouter,
  UpstreamAccountCreatePage,
  upstreamAccountCreateMetaBase,
} from './UpstreamAccountCreatePage.story-common'

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: 'Account Pool/Pages/Upstream Account Create/API Key',
} satisfies Meta<typeof UpstreamAccountCreatePage>

export default meta

type Story = StoryObj<typeof meta>

export const NameConflict: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=apiKey',
        state: {
          draft: {
            apiKey: {
              displayName: ' team key - staging ',
              groupName: 'staging',
              note: 'Conflicts with an existing API Key account name.',
              apiKeyValue: 'sk-storybookduplicate1234',
              primaryLimit: '120',
              secondaryLimit: '500',
              limitUnit: 'requests',
            },
          },
        },
      }}
    />
  ),
}

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts/new',
        search: '?mode=apiKey',
        state: {
          draft: {
            apiKey: {
              displayName: 'Staging Key',
              groupName: 'staging',
              apiKeyValue: 'sk-storybookstaging1234',
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /create api key account/i })).toBeDisabled()
  },
}

export const InvalidUpstreamUrl: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=apiKey" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText(/display name/i), 'Gateway Key')
    await userEvent.type(canvas.getByLabelText(/^api key$/i), 'sk-gateway')
    await userEvent.type(canvas.getByLabelText(/upstream base url/i), 'proxy.example.com/gateway')
    await expect(canvas.getByText(/absolute http\(s\) url|http\(s\) 的绝对 url/i)).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /create api key account/i })).toBeDisabled()
  },
}
