import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect, waitFor } from 'storybook/test'
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

export const CompactLongLabels: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
}

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
      name: /编辑路由设置|edit routing settings/i,
    })
    await userEvent.click(editButton)
    const dialog = documentScope.getByRole('dialog', {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    })
    await expect(dialog).toBeInTheDocument()
    const generateButton = within(dialog).getByRole('button', { name: /生成密钥|generate key/i })
    await expect(generateButton).toBeInTheDocument()
    await expect(
      within(dialog).getByLabelText(/优先队列同步间隔|priority sync interval/i),
    ).toBeInTheDocument()
    await expect(
      within(dialog).getByLabelText(/次级队列同步间隔|secondary sync interval/i),
    ).toBeInTheDocument()
    await expect(
      within(dialog).getByLabelText(/优先可用账号上限|priority available account cap/i),
    ).toBeInTheDocument()
    await userEvent.click(generateButton)
    const input = within(dialog).getByPlaceholderText(/粘贴新的号池 API Key|paste a new pool api key/i) as HTMLInputElement
    await expect(input.value).toMatch(/^cvm-[0-9a-f]{32}$/)
    await userEvent.click(within(dialog).getByRole('button', { name: /取消|cancel/i }))
    await userEvent.click(await canvas.findByRole('button', { name: /编辑路由设置|edit routing settings/i }))
    const reopenedDialog = documentScope.getByRole('dialog', {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    })
    const reopenedInput = within(reopenedDialog).getByPlaceholderText(/粘贴新的号池 API Key|paste a new pool api key/i) as HTMLInputElement
    await expect(reopenedInput.value).toBe('')
  },
}

export const RoutingDialogMaintenanceOnlySave: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)

    await userEvent.click(
      await canvas.findByRole('button', { name: /编辑路由设置|edit routing settings/i }),
    )
    const dialog = await documentScope.findByRole('dialog', {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    })
    const primaryInput = within(dialog).getByLabelText(
      /优先队列同步间隔|priority sync interval/i,
    ) as HTMLInputElement
    const secondaryInput = within(dialog).getByLabelText(
      /次级队列同步间隔|secondary sync interval/i,
    ) as HTMLInputElement
    const capInput = within(dialog).getByLabelText(
      /优先可用账号上限|priority available account cap/i,
    ) as HTMLInputElement
    const apiKeyInput = within(dialog).getByPlaceholderText(
      /粘贴新的号池 API Key|paste a new pool api key/i,
    ) as HTMLInputElement
    const saveButton = within(dialog).getByRole('button', { name: /保存设置|save settings/i })

    await expect(primaryInput.value).toBe('300')
    await expect(secondaryInput.value).toBe('1800')
    await expect(capInput.value).toBe('100')
    await expect(apiKeyInput.value).toBe('')
    await expect(saveButton).toBeDisabled()

    await userEvent.clear(primaryInput)
    await userEvent.type(primaryInput, '600')
    await userEvent.clear(secondaryInput)
    await userEvent.type(secondaryInput, '2400')
    await userEvent.clear(capInput)
    await userEvent.type(capInput, '42')

    await expect(saveButton).toBeEnabled()
    await userEvent.click(saveButton)

    await waitFor(() => {
      expect(
        documentScope.queryByRole('dialog', {
          name: /高级路由与同步设置|advanced routing & sync settings/i,
        }),
      ).not.toBeInTheDocument()
    })
    await expect(canvas.getByText(/^(10m|10 分钟)$/i)).toBeInTheDocument()
    await expect(canvas.getByText(/^(40m|40 分钟)$/i)).toBeInTheDocument()
    await expect(canvas.getByText(/^(Top 42 accounts|前 42 个账号)$/i)).toBeInTheDocument()
  },
}

export const RoutingDialogValidation: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)

    await userEvent.click(
      await canvas.findByRole('button', { name: /编辑路由设置|edit routing settings/i }),
    )
    const dialog = await documentScope.findByRole('dialog', {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    })
    const primaryInput = within(dialog).getByLabelText(
      /优先队列同步间隔|priority sync interval/i,
    ) as HTMLInputElement
    const secondaryInput = within(dialog).getByLabelText(
      /次级队列同步间隔|secondary sync interval/i,
    ) as HTMLInputElement
    const saveButton = within(dialog).getByRole('button', { name: /保存设置|save settings/i })

    await userEvent.clear(primaryInput)
    await userEvent.type(primaryInput, '600')
    await userEvent.clear(secondaryInput)
    await userEvent.type(secondaryInput, '300')

    await expect(
      within(dialog).getByText(
        /次级队列同步间隔必须大于等于优先队列同步间隔|secondary sync interval must be greater than or equal to the priority sync interval/i,
      ),
    ).toBeInTheDocument()
    await expect(saveButton).toBeDisabled()
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

export const TagFilterAllMatch: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)
    const filterTrigger = await canvas.findByRole('button', {
      name: /按标签筛选账号|filter accounts by tags/i,
    })
    await userEvent.click(filterTrigger)
    await userEvent.click(await documentScope.findByText(/^vip$/i))
    await userEvent.click(await documentScope.findByText(/^burst-safe$/i))
    await expect(canvas.getByText(/Codex Pro - Tokyo/i)).toBeInTheDocument()
    await expect(canvas.queryByText(/Team key - staging/i)).not.toBeInTheDocument()
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
