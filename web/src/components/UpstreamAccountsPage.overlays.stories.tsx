import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'
import { duplicateReasons } from './UpstreamAccountsPage.story-data'
import { SystemNotificationProvider } from './ui/system-notifications'

const meta = {
  title: 'Account Pool/Pages/Upstream Accounts/Overlays',
  component: UpstreamAccountsPage,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <SystemNotificationProvider>
          <StorybookUpstreamAccountsMock>
            <Story />
          </StorybookUpstreamAccountsMock>
        </SystemNotificationProvider>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountsPage>

export default meta

type Story = StoryObj<typeof meta>

export const DetailDrawer: Story = {
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
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    await expect(documentScope.getByRole('dialog', { name: /Codex Pro - Tokyo/i })).toBeInTheDocument()
  },
}

export const DeleteConfirmation: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts',
        state: {
          selectedAccountId: 101,
          openDetail: true,
          openDeleteConfirm: true,
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    await expect(documentScope.getByRole('alertdialog')).toBeInTheDocument()
    await expect(documentScope.getByText(/确认删除 Codex Pro - Tokyo|delete Codex Pro - Tokyo/i)).toBeInTheDocument()
  },
}

export const DeleteFailure: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts',
        state: {
          selectedAccountId: 101,
          openDetail: true,
          openDeleteConfirm: true,
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    const confirmDialog = await documentScope.findByRole('alertdialog')
    await userEvent.click(within(confirmDialog).getByRole('button', { name: /确认删除|delete account/i }))
    await expect(within(dialog).getByText(/database is locked/i)).toBeInTheDocument()
    await expect(documentScope.queryByRole('alertdialog')).not.toBeInTheDocument()
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
    const dialog = documentScope.getByRole('dialog', { name: /编辑号池路由密钥|update pool routing key/i })
    await expect(dialog).toBeInTheDocument()
    const generateButton = within(dialog).getByRole('button', { name: /生成密钥|generate key/i })
    await expect(generateButton).toBeInTheDocument()
    await userEvent.click(generateButton)
    const input = within(dialog).getByPlaceholderText(/粘贴新的号池 API Key|paste a new pool api key/i) as HTMLInputElement
    await expect(input.value).toMatch(/^cvm-[0-9a-f]{32}$/)
    await userEvent.click(within(dialog).getByRole('button', { name: /取消|cancel/i }))
    await userEvent.click(await canvas.findByRole('button', { name: /编辑号池密钥|edit pool key/i }))
    const reopenedDialog = documentScope.getByRole('dialog', { name: /编辑号池路由密钥|update pool routing key/i })
    const reopenedInput = within(reopenedDialog).getByPlaceholderText(/粘贴新的号池 API Key|paste a new pool api key/i) as HTMLInputElement
    await expect(reopenedInput.value).toBe('')
  },
}

export const DetailDrawerGroupNotes: Story = {
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
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    await userEvent.click(
      await documentScope.findByRole('button', {
        name: /编辑分组备注|edit group note/i,
      }),
    )
    await expect(
      documentScope.getByRole('dialog', { name: /编辑分组备注|edit group note/i }),
    ).toBeInTheDocument()
    await expect(documentScope.getByText(/production/i)).toBeInTheDocument()
  },
}

export const DetailDrawerApiKeyInvalidUpstreamUrl: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: '/account-pool/upstream-accounts',
        state: {
          selectedAccountId: 102,
          openDetail: true,
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = documentScope.getByRole('dialog', { name: /Team key - staging/i })
    const field = within(dialog).getByLabelText(/upstream base url/i)
    await userEvent.clear(field)
    await userEvent.type(field, 'https://proxy.example.com/gateway?team=staging')
    await expect(documentScope.getByText(/cannot include a query string or fragment|不能包含查询串或片段/i)).toBeInTheDocument()
    await expect(within(dialog).getByRole('button', { name: /save changes/i })).toBeDisabled()
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
