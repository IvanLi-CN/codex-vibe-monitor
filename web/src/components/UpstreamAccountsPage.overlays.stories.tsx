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

function detailRouteEntry(accountId: number, state?: Record<string, unknown>) {
  return {
    pathname: '/account-pool/upstream-accounts',
    search: `?upstreamAccountId=${accountId}`,
    state,
  }
}

export const DetailDrawer: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101)}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    await expect(within(dialog).getByRole('tab', { name: /概览|overview/i })).toHaveAttribute('aria-selected', 'true')
    await expect(within(dialog).getByText(/最近成功同步|last successful sync/i)).toBeInTheDocument()
    await expect(within(dialog).getByText(/5 小时窗口|5h window/i)).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('tab', { name: /调用记录|records/i }))
    await expect(within(dialog).getByText(/查看这个上游账号最近保留的调用记录|latest retained invocations routed to this upstream account/i)).toBeInTheDocument()
    await expect(within(dialog).getByText(/gpt-5\.4/i)).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('tab', { name: /路由|routing/i }))
    await expect(within(dialog).getByText(/最终生效规则|effective routing rule/i)).toBeInTheDocument()
    await expect(within(dialog).getByText(/sticky-pool/i)).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('tab', { name: /健康与事件|health & events/i }))
    await expect(within(dialog).getByText(/最近账号动作|latest account action/i)).toBeInTheDocument()
    await expect(within(dialog).getByText(/Weekly cap exhausted; traffic was moved to a sibling Tokyo lane\./i)).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('tab', { name: /编辑|edit/i }))
    await expect(within(dialog).getByLabelText(/显示名称|display name/i)).toBeInTheDocument()
  },
}

export const DetailDrawerStickyHistory: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101)}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    await userEvent.click(within(dialog).getByRole('tab', { name: /路由|routing/i }))
    await userEvent.click(
      within(dialog).getAllByRole('button', { name: /打开全部调用记录|open full call history/i })[0],
    )
    await expect(documentScope.getByText(/019ce3a1-6787-7910-b0fd-c246d6f6a901/i)).toBeInTheDocument()
    await expect(documentScope.getByText(/gpt-5\.4/i)).toBeInTheDocument()
  },
}

export const MissingWindowPlaceholders: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(102)}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = await documentScope.findByRole('dialog', { name: /Team key - missing weekly limit/i })
    await expect(within(dialog).getByText(/18 requests/i)).toBeInTheDocument()
    expect(within(dialog).getAllByText('-').length).toBeGreaterThanOrEqual(4)
    await expect(within(dialog).queryByText(/还没有额度历史|No quota history yet/i)).not.toBeInTheDocument()
  },
}

export const DeleteConfirmation: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
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
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
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

export const DeleteSuccessClosesDrawer: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    const confirmDialog = await documentScope.findByRole('alertdialog')
    await userEvent.click(within(confirmDialog).getByRole('button', { name: /确认删除|delete account/i }))
    await waitFor(() => {
      expect(documentScope.queryByRole('dialog', { name: /Codex Pro - Tokyo/i })).toBeNull()
    })
    await expect(documentScope.queryByRole('alertdialog')).toBeNull()
    await expect(documentScope.getByRole('heading', { name: /upstream accounts|上游账号/i })).toBeInTheDocument()
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

export const RoutingDialogTimeoutSettings: Story = {
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
    const responsesFirstByteInput = within(dialog).getByLabelText(
      /一般请求响应体首字超时|standard response first byte timeout/i,
    ) as HTMLInputElement
    const compactFirstByteInput = within(dialog).getByLabelText(
      /压缩请求响应体首字超时|compact response first byte timeout/i,
    ) as HTMLInputElement
    const responsesStreamInput = within(dialog).getByLabelText(
      /一般请求流结束超时|standard stream completion timeout/i,
    ) as HTMLInputElement
    const compactStreamInput = within(dialog).getByLabelText(
      /压缩请求流结束超时|compact stream completion timeout/i,
    ) as HTMLInputElement
    const saveButton = within(dialog).getByRole('button', { name: /保存设置|save settings/i })

    await expect(responsesFirstByteInput.value).toBe('120')
    await expect(compactFirstByteInput.value).toBe('300')
    await expect(responsesStreamInput.value).toBe('300')
    await expect(compactStreamInput.value).toBe('300')

    await userEvent.clear(responsesFirstByteInput)
    await userEvent.type(responsesFirstByteInput, '180')
    await userEvent.clear(compactFirstByteInput)
    await userEvent.type(compactFirstByteInput, '420')
    await userEvent.clear(responsesStreamInput)
    await userEvent.type(responsesStreamInput, '360')
    await userEvent.clear(compactStreamInput)
    await userEvent.type(compactStreamInput, '540')

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
          /一般请求响应体首字超时|standard response first byte timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('180')
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /压缩请求响应体首字超时|compact response first byte timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('420')
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /一般请求流结束超时|standard stream completion timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('360')
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /压缩请求流结束超时|compact stream completion timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe('540')
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

export const CompactSupportDetailDrawer: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101)}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)

    await expect(await canvas.findByText(/Compact 不支持|Compact unsupported/i)).toBeInTheDocument()
    const dialog = await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    await userEvent.click(within(dialog).getByRole('tab', { name: /健康与事件|health & events/i }))
    await expect(within(dialog).getByText(/Compact 支持|Compact support/i)).toBeInTheDocument()
    await expect(within(dialog).getByText(/不支持|unsupported/i)).toBeInTheDocument()
    await expect(
      within(dialog).getByText(/No available channel for model gpt-5\.4-openai-compact/i),
    ).toBeInTheDocument()
  },
}
export const DetailDrawerGroupNotes: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101)}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = await documentScope.findByRole('dialog', { name: /Codex Pro - Tokyo/i })
    await userEvent.click(within(dialog).getByRole('tab', { name: /编辑|edit/i }))
    await userEvent.click(
      await within(dialog).findByRole('button', {
        name: /编辑分组设置|edit group settings|编辑分组备注|edit group note/i,
      }),
    )
    await expect(
      documentScope.getByRole('dialog', { name: /分组设置|group settings|分组备注|group note/i }),
    ).toBeInTheDocument()
    await expect(documentScope.getByText(/production/i)).toBeInTheDocument()
  },
}

export const DetailDrawerApiKeyInvalidUpstreamUrl: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(102)}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const dialog = documentScope.getByRole('dialog', { name: /Team key - staging/i })
    await userEvent.click(within(dialog).getByRole('tab', { name: /编辑|edit/i }))
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
      initialEntry={detailRouteEntry(101)}
    />
  ),
}
