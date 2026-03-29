import { useEffect } from 'react'
import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, waitFor, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'
import { SystemNotificationProvider } from './ui/system-notifications'

const UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY =
  'codex-vibe-monitor.account-pool.upstream-accounts.filters'

const meta = {
  title: 'Account Pool/Pages/Upstream Accounts/List',
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

function PersistedFiltersStoryRouter() {
  useEffect(() => {
    if (typeof window === 'undefined') {
      return
    }
    const previousValue = window.localStorage.getItem(UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY)
    window.localStorage.setItem(
      UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
      JSON.stringify({
        workStatus: ['rate_limited'],
        enableStatus: ['enabled'],
        healthStatus: ['normal'],
        tagIds: [],
        groupFilter: {
          mode: 'search',
          query: 'prod',
        },
      }),
    )
    return () => {
      if (previousValue == null) {
        window.localStorage.removeItem(UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY)
        return
      }
      window.localStorage.setItem(UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY, previousValue)
    }
  }, [])

  return <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
}

async function chooseSelectOption(
  canvasElement: HTMLElement,
  triggerMatcher: RegExp,
  optionMatcher: RegExp,
) {
  const documentScope = within(canvasElement.ownerDocument.body)
  const trigger = await documentScope.findByRole('combobox', {
    name: triggerMatcher,
  })
  await userEvent.click(trigger)
  const option = await documentScope.findByRole('option', {
    name: optionMatcher,
  })
  await userEvent.click(option)
}

async function chooseCommandOptions(
  canvasElement: HTMLElement,
  triggerMatcher: RegExp,
  optionMatchers: RegExp[],
) {
  const documentScope = within(canvasElement.ownerDocument.body)
  const trigger = await documentScope.findByRole('combobox', {
    name: triggerMatcher,
  })
  await userEvent.click(trigger)
  for (const optionMatcher of optionMatchers) {
    const option = await documentScope.findByText(optionMatcher)
    await userEvent.click(option)
  }
}

async function clickCheckboxByLabel(
  canvasElement: HTMLElement,
  matcher: RegExp,
) {
  const documentScope = within(canvasElement.ownerDocument.body)
  const checkbox = await documentScope.findByRole('checkbox', {
    name: matcher,
  })
  await userEvent.click(checkbox)
}

async function choosePageSize(canvasElement: HTMLElement, pageSize: number) {
  await chooseSelectOption(
    canvasElement,
    /每页|page size/i,
    new RegExp(`^${pageSize}$`, 'i'),
  )
}

async function findAccountRow(canvasElement: HTMLElement, matcher: RegExp) {
  const documentScope = within(canvasElement.ownerDocument.body)
  return documentScope.findByRole('button', { name: matcher })
}

function findRowBadge(row: HTMLElement, matcher: RegExp) {
  const badges = Array.from(
    row.querySelectorAll<HTMLElement>('div.inline-flex.items-center.rounded-full.border'),
  )
  const badge = badges.find((candidate) => matcher.test(candidate.textContent?.trim() ?? ''))
  if (!badge) {
    throw new Error(`missing badge: ${matcher}`)
  }
  return badge
}

function expectBadgeAlignment(reference: HTMLElement, candidate: HTMLElement) {
  const referenceRect = reference.getBoundingClientRect()
  const candidateRect = candidate.getBoundingClientRect()
  expect(Math.abs(candidateRect.top - referenceRect.top)).toBeLessThanOrEqual(0.5)
  expect(Math.abs(candidateRect.height - referenceRect.height)).toBeLessThanOrEqual(0.5)
}

export const Operational: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement, step }) => {
    const canvasScope = within(canvasElement)
    await step('keep the routing summary card free of advanced setting tiles', async () => {
      await expect(
        await canvasScope.findByText(/current pool api key|当前号池 API Key/i),
      ).toBeInTheDocument()
      await expect(
        await canvasScope.findByRole('button', {
          name: /edit routing settings|编辑路由设置/i,
        }),
      ).toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/priority sync interval|优先队列同步间隔/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/secondary sync interval|次级队列同步间隔/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/priority available account cap|优先可用账号上限/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/standard response first byte timeout|一般请求响应体首字超时/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/compact response first byte timeout|压缩请求响应体首字超时/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/standard stream completion timeout|一般请求流结束超时/i),
      ).not.toBeInTheDocument()
      await expect(
        canvasScope.queryByText(/compact stream completion timeout|压缩请求流结束超时/i),
      ).not.toBeInTheDocument()
    })
  },
}

export const DenseRoster: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement, step }) => {
    await step('show more rows per page', async () => {
      await choosePageSize(canvasElement, 50)
    })
  },
}

export const CompactLongLabels: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement, step }) => {
    await step('keeps unsupported compact badges aligned and hides supported markers', async () => {
      const oauthRow = await findAccountRow(
        canvasElement,
        /Codex Pro - Tokyo enterprise rotation account with a deliberately long roster title/i,
      )
      const oauthKindBadge = findRowBadge(oauthRow, /^OAuth$/i)
      const oauthPlanBadge = findRowBadge(oauthRow, /^pro$/i)
      const compactUnsupportedBadge = findRowBadge(oauthRow, /^Compact (不支持|unsupported)$/i)
      const oauthOverflowBadge = findRowBadge(oauthRow, /^\+1$/)
      const oauthVisibleTagBadge = findRowBadge(oauthRow, /^prod-apac$/i)

      expectBadgeAlignment(oauthKindBadge, compactUnsupportedBadge)
      expectBadgeAlignment(oauthPlanBadge, compactUnsupportedBadge)
      expectBadgeAlignment(oauthVisibleTagBadge, oauthOverflowBadge)

      const apiKeyRow = await findAccountRow(
        canvasElement,
        /Team key - staging/i,
      )
      expect(() => findRowBadge(apiKeyRow, /^Compact (可用|available)$/i)).toThrow()
    })
  },
}

export const MissingWindowPlaceholders: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement, step }) => {
    await step('render missing secondary windows with weak dash placeholders instead of 0%', async () => {
      const row = await findAccountRow(
        canvasElement,
        /Team key - missing weekly limit/i,
      )
      expect(within(row).getAllByText('-').length).toBeGreaterThanOrEqual(3)
      await expect(within(row).queryByText(/^7D$/i)).not.toBeInTheDocument()
      await expect(within(row).queryByText(/^0%$/i)).not.toBeInTheDocument()
      await expect(within(row).getByText(/18 requests/i)).toBeInTheDocument()
    })
  },
}

export const StatusFilters: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)
    await chooseCommandOptions(
      canvasElement,
      /工作状态|work status/i,
      [/限流|rate limited/i, /工作|working/i],
    )
    await chooseCommandOptions(
      canvasElement,
      /启用状态|enable status/i,
      [/启用|enabled/i],
    )
    await chooseCommandOptions(
      canvasElement,
      /账号状态|account health/i,
      [/正常|normal/i],
    )
    await expect(
      await canvas.findByText(/Team key - staging/i),
    ).toBeInTheDocument()
    await expect(
      await canvas.findByText(/Codex Pro - Tokyo/i),
    ).toBeInTheDocument()
  },
}

export const UnavailableWorkStatusFilter: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)
    await chooseCommandOptions(
      canvasElement,
      /工作状态|work status/i,
      [/不可用|unavailable/i],
    )

    await expect(
      await canvas.findByText(/Needs reauth unavailable work status/i),
    ).toBeInTheDocument()
    await expect(
      await canvas.findByText(/Upstream unavailable work status/i),
    ).toBeInTheDocument()
    await expect(
      await canvas.findByText(/Upstream rejected unavailable work status/i),
    ).toBeInTheDocument()
    await expect(
      canvas.queryByText(/Rate limited filter control/i),
    ).not.toBeInTheDocument()
  },
}

export const BulkSelection: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)
    await clickCheckboxByLabel(canvasElement, /选择当前页|select current page/i)
    await expect(
      await canvas.findByText(
        /已跨页选中 \d+ 个账号|\d+ accounts selected across pages/i,
      ),
    ).toBeInTheDocument()
  },
}

export const BulkSyncSuccessAutoHide: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)

    await userEvent.click(
      await documentScope.findByRole('checkbox', { name: /select existing oauth/i }),
    )
    await userEvent.click(
      await documentScope.findByRole('checkbox', { name: /select team key - staging/i }),
    )
    const syncButton = await canvas.findByRole('button', {
      name: /sync selected|批量同步/i,
    })

    await userEvent.click(syncButton)
    const progressTitle = await canvas.findByText(/bulk sync progress|批量同步进度/i)
    await expect(progressTitle).toBeInTheDocument()
    await expect(progressTitle.closest('.fixed')).not.toBeNull()

    await waitFor(() => {
      expect(
        canvas.queryByText(/bulk sync progress|批量同步进度/i),
      ).not.toBeInTheDocument()
    })

    await expect(syncButton).toBeEnabled()
    await expect(
      await canvas.findByText(
        /2 accounts selected across pages|已跨页选中 2 个账号/i,
      ),
    ).toBeInTheDocument()
  },
}

export const BulkSyncFailureDismiss: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    const documentScope = within(canvasElement.ownerDocument.body)

    await userEvent.click(
      await documentScope.findByRole('checkbox', { name: /select existing oauth/i }),
    )
    await userEvent.click(
      await documentScope.findByRole('checkbox', { name: /select team key - staging/i }),
    )
    const syncButton = await canvas.findByRole('button', {
      name: /sync selected|批量同步/i,
    })

    await userEvent.click(syncButton)
    const progressTitle = await canvas.findByText(/bulk sync progress|批量同步进度/i)
    await expect(progressTitle).toBeInTheDocument()
    await expect(progressTitle.closest('.fixed')).not.toBeNull()
    await expect(
      await canvas.findByText(/refresh token already rotated/i),
    ).toBeInTheDocument()

    const dismissButton = await canvas.findByRole('button', {
      name: /dismiss|收起/i,
    })
    await expect(syncButton).toBeEnabled()
    await userEvent.click(dismissButton)

    await waitFor(() => {
      expect(
        canvas.queryByText(/bulk sync progress|批量同步进度/i),
      ).not.toBeInTheDocument()
    })

    await expect(
      await canvas.findByText(
        /2 accounts selected across pages|已跨页选中 2 个账号/i,
      ),
    ).toBeInTheDocument()
  },
}

export const TagFilterAllMatch: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
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
    await expect(
      canvas.queryByText(/Team key - staging/i),
    ).not.toBeInTheDocument()
  },
}

export const PersistedRosterFilters: Story = {
  render: () => <PersistedFiltersStoryRouter />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)

    await expect(
      await canvas.findByText(/Codex Pro - London/i),
    ).toBeInTheDocument()
    await expect(
      canvas.queryByText(/Team key - staging/i),
    ).not.toBeInTheDocument()
    await expect(
      await canvas.findByRole('button', {
        name: /work status/i,
      }),
    ).toHaveTextContent(/rate limited/i)
    await expect(
      await canvas.findByRole('button', {
        name: /enable status/i,
      }),
    ).toHaveTextContent(/enabled/i)
    await expect(
      await canvas.findByRole('button', {
        name: /account health/i,
      }),
    ).toHaveTextContent(/normal/i)
    await expect(
      await canvas.findByRole('button', {
        name: /group/i,
      }),
    ).toHaveTextContent(/prod/i)
  },
}

export const AvailabilityBadges: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)

    const workingCell = await canvas.findByText(/Availability working badge/i)
    const workingRow = workingCell.closest('tr')
    await expect(workingRow).toHaveTextContent(/工作 3|Working 3/i)

    const idleCell = await canvas.findByText(/Availability idle badge/i)
    const idleRow = idleCell.closest('tr')
    await expect(idleRow).toHaveTextContent(/空闲|Idle/i)

    const rateLimitedCell = await canvas.findByText(
      /Availability rate limited visible/i,
    )
    const rateLimitedRow = rateLimitedCell.closest('tr')
    await expect(rateLimitedRow).toHaveTextContent(/限流|Rate limited/i)
    await expect(rateLimitedRow).not.toHaveTextContent(/工作 \d+|Working \d+/i)
    await expect(rateLimitedRow).not.toHaveTextContent(/空闲|Idle/i)

    const unavailableCell = await canvas.findByText(
      /Availability unavailable hidden/i,
    )
    const unavailableRow = unavailableCell.closest('tr')
    await expect(unavailableRow).not.toHaveTextContent(/工作|Working/i)
    await expect(unavailableRow).not.toHaveTextContent(/空闲|Idle/i)
  },
}

export const QuotaExhaustedOauth: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const row = await documentScope.findByRole('button', {
      name: /Quota exhausted OAuth routing state/i,
    })

    await expect(row).toHaveTextContent(/限流|Rate limited/i)
    await expect(row).not.toHaveTextContent(/上游拒绝|Upstream rejected/i)

    await userEvent.click(row)

    await expect(
      await documentScope.findByText(/恢复仍被阻止|Recovery blocked/i),
    ).toBeInTheDocument()
    await expect(
      await documentScope.findByText(
        /最新额度快照仍显示限制窗口已耗尽|latest usage snapshot still shows an exhausted upstream usage limit window/i,
      ),
    ).toBeInTheDocument()
    await expect(
      documentScope.queryByText(/^上游拒绝$|^Upstream rejected$/i),
    ).not.toBeInTheDocument()
  },
}

export const OauthRetryTerminalState: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const row = await documentScope.findByRole('button', {
      name: /Retry refresh failure settled as needs reauth/i,
    })

    await expect(row).toHaveTextContent(/需要重新授权|Needs reauth/i)
    await expect(row).not.toHaveTextContent(/同步中|Syncing/i)

    await userEvent.click(row)

    const dialog = await documentScope.findByRole('dialog', {
      name: /Retry refresh failure settled as needs reauth/i,
    })
    await expect(await within(dialog).findByText(/^Unavailable$/i)).toBeInTheDocument()
    await expect(dialog).toHaveTextContent(/需要重新授权|Needs reauth/i)
    await expect(dialog).not.toHaveTextContent(/同步中|Syncing/i)
    await expect(
      await documentScope.findByText(
        /Authentication token has been invalidated, please sign in again/i,
      ),
    ).toBeInTheDocument()
  },
}

export const UpstreamRejected402: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body)
    const row = await documentScope.findByRole('button', {
      name: /Workspace deactivated 402 routing state/i,
    })

    await expect(row).toHaveTextContent(/上游拒绝|Upstream rejected/i)
    await expect(row).not.toHaveTextContent(/其它异常|Other error/i)
    await expect(row).toHaveTextContent(/HTTP 402/i)

    await userEvent.click(row)

    const dialog = await documentScope.findByRole('dialog', {
      name: /Workspace deactivated 402 routing state/i,
    })
    await expect(await within(dialog).findByText(/^Unavailable$/i)).toBeInTheDocument()
    await expect(
      await documentScope.findByText(/^上游拒绝$|^Upstream rejected$/i),
    ).toBeInTheDocument()
    await expect(
      await documentScope.findByText(
        /Plan or billing rejected upstream access \(402\)|上游因套餐或计费拒绝访问（402）/i,
      ),
    ).toBeInTheDocument()
    await expect(
      await documentScope.findByText(/deactivated_workspace/i),
    ).toBeInTheDocument()
  },
}
