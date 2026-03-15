import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect } from 'storybook/test'
import { SystemNotificationProvider } from './ui/system-notifications'
import { I18nProvider } from '../i18n'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'
import { duplicateReasons } from './UpstreamAccountsPage.story-data'

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
    await expect(documentScope.getByText(/删除这个上游账号|delete this upstream account/i)).toBeInTheDocument()
    await expect(documentScope.getByText(/不会保留恢复副本|does not keep a recovery copy/i)).toBeInTheDocument()
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

export const CreateAccount: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
}

export const CreateAccountApiKeyInvalidUpstreamUrl: Story = {
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

export const CreateAccountOauthReady: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.type(canvas.getByLabelText(/display name/i), 'Codex Pro - Manual')
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByRole('button', { name: /copy oauth url/i })).toBeInTheDocument()
    await expect(canvas.getByLabelText(/callback url/i)).toBeInTheDocument()
  },
}

export const CreateAccountBatchOauthReady: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await userEvent.click(canvas.getByRole('button', { name: /generate oauth url/i }))
    await expect(canvas.getByDisplayValue(/https:\/\/auth\.openai\.com\/authorize/i)).toBeInTheDocument()
    await expect(canvas.getByRole('button', { name: /complete oauth login/i })).toBeInTheDocument()
  },
}

export const DetailDrawerGroupNotes: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    await userEvent.click(
      await canvas.findByRole('button', {
        name: /打开详情/i,
      }),
    )
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
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    const openButtons = await canvas.findAllByRole('button', {
      name: /打开详情/i,
    })
    await userEvent.click(openButtons[1])
    const dialog = documentScope.getByRole('dialog', { name: /Team key - staging/i })
    const field = within(dialog).getByLabelText(/upstream base url/i)
    await userEvent.clear(field)
    await userEvent.type(field, 'https://proxy.example.com/gateway?team=staging')
    await expect(documentScope.getByText(/cannot include a query string or fragment|不能包含查询串或片段/i)).toBeInTheDocument()
    await expect(within(dialog).getByRole('button', { name: /save changes/i })).toBeDisabled()
  },
}

export const CreateAccountBatchGroupNoteDraft: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=batchOauth" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const doc = canvasElement.ownerDocument
    const trigger = canvas.getAllByRole('combobox')[0]
    await userEvent.click(trigger)

    const searchInput = doc.body.querySelector('[cmdk-input]')
    if (!(searchInput instanceof HTMLInputElement)) {
      throw new Error('missing group combobox search input')
    }
    await userEvent.type(searchInput, 'new-team')

    const createOption = Array.from(doc.body.querySelectorAll('[cmdk-item]')).find((candidate) =>
      (candidate.textContent || '').toLowerCase().includes('new-team'),
    )
    if (!(createOption instanceof HTMLElement)) {
      throw new Error('missing create option for new-team')
    }
    await userEvent.click(createOption)

    const documentScope = within(doc.body)
    await userEvent.click(
      await documentScope.findByRole('button', {
        name: /编辑分组备注|edit group note/i,
      }),
    )
    await expect(
      documentScope.getByRole('dialog', { name: /编辑分组备注|edit group note/i }),
    ).toBeInTheDocument()
    await expect(documentScope.getByText(/new-team/i)).toBeInTheDocument()
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
