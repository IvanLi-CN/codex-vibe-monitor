import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect } from 'storybook/test'
import { I18nProvider } from '../i18n'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
  duplicateReasons,
} from './UpstreamAccountsPage.story-helpers'

const meta = {
  title: 'Account Pool/Pages/Upstream Accounts',
  component: UpstreamAccountsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorybookUpstreamAccountsMock>
          <Story />
        </StorybookUpstreamAccountsMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountsPage>

export default meta

type Story = StoryObj<typeof meta>

export const Operational: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
}

export const DetailDrawer: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    const openButton = await canvas.findByRole('button', {
      name: /打开详情/i,
    })
    await userEvent.click(openButton)
    await expect(documentScope.getByRole('dialog', { name: /Codex Pro - Tokyo/i })).toBeInTheDocument()
  },
}

export const RoutingDialog: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    const editButton = await canvas.findByRole('button', {
      name: /编辑号池密钥|edit pool key/i,
    })
    await userEvent.click(editButton)
    await expect(
      documentScope.getByRole('dialog', { name: /编辑号池路由密钥|update pool routing key/i }),
    ).toBeInTheDocument()
  },
}

export const DuplicateOauthWarning: Story = {
  name: 'Duplicate OAuth Warning',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts',
        state: {
          selectedAccountId: 101,
          duplicateWarning: {
            accountId: 101,
            displayName: 'Codex Pro - Tokyo',
            peerAccountIds: [103],
            reasons: [...duplicateReasons],
          },
        },
      }}
    />
  ),
}

export const DuplicateOauthDetail: Story = {
  name: 'Duplicate OAuth Detail',
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts',
        state: {
          selectedAccountId: 101,
          openDetail: true,
        },
      }}
    />
  ),
}
