import type { Meta, StoryObj } from '@storybook/react-vite'
import { userEvent, within, expect, waitFor } from 'storybook/test'
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

    await userEvent.click(
      await canvas.findByRole('button', { name: /编辑路由设置|edit routing settings/i }),
    )
    const reopenedDialog = await documentScope.findByRole('dialog', {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    })
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /优先队列同步间隔|priority sync interval/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('600')
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /次级队列同步间隔|secondary sync interval/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('2400')
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /优先可用账号上限|priority available account cap/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('42')
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
