import type { Meta, StoryObj } from '@storybook/react-vite'
import { expect, userEvent, within } from 'storybook/test'
import { I18nProvider } from '../i18n'
import UpstreamAccountsPage from '../pages/account-pool/UpstreamAccounts'
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from './UpstreamAccountsPage.story-helpers'
import { SystemNotificationProvider } from './ui/system-notifications'

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

async function chooseSelectOption(
  canvasElement: HTMLElement,
  triggerMatcher: RegExp,
  optionMatcher: RegExp,
) {
  const documentScope = within(canvasElement.ownerDocument.body)
  const trigger = await documentScope.findByRole('combobox', { name: triggerMatcher })
  await userEvent.click(trigger)
  const option = await documentScope.findByRole('option', { name: optionMatcher })
  await userEvent.click(option)
}

async function clickCheckboxByLabel(canvasElement: HTMLElement, matcher: RegExp) {
  const documentScope = within(canvasElement.ownerDocument.body)
  const checkbox = await documentScope.findByRole('checkbox', { name: matcher })
  await userEvent.click(checkbox)
}

async function choosePageSize(canvasElement: HTMLElement, pageSize: number) {
  await chooseSelectOption(
    canvasElement,
    /每页|page size/i,
    new RegExp(`^${pageSize}$`, 'i'),
  )
}

export const Operational: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
}

export const DenseRoster: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement, step }) => {
    await step('show more rows per page', async () => {
      await choosePageSize(canvasElement, 50)
    })
  },
}

export const CompactLongLabels: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
}

export const StatusFilters: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)
    await chooseSelectOption(canvasElement, /工作状态|work status/i, /限流|rate limited/i)
    await chooseSelectOption(canvasElement, /启用状态|enable status/i, /启用|enabled/i)
    await chooseSelectOption(canvasElement, /账号状态|account health/i, /正常|normal/i)
    await expect(await canvas.findByText(/Team key - staging/i)).toBeInTheDocument()
    await expect(canvas.queryByText(/Codex Pro - Tokyo/i)).not.toBeInTheDocument()
  },
}

export const BulkSelection: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)
    await clickCheckboxByLabel(canvasElement, /选择当前页|select current page/i)
    await expect(
      await canvas.findByText(/已跨页选中 \d+ 个账号|\d+ accounts selected across pages/i),
    ).toBeInTheDocument()
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

export const AvailabilityBadges: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement)
    await choosePageSize(canvasElement, 50)

    const workingCell = await canvas.findByText(/Availability working badge/i)
    const workingRow = workingCell.closest('tr')
    await expect(workingRow).toHaveTextContent(/工作 3|Working 3/i)

    const idleCell = await canvas.findByText(/Availability idle badge/i)
    const idleRow = idleCell.closest('tr')
    await expect(idleRow).toHaveTextContent(/空闲|Idle/i)

    const rateLimitedCell = await canvas.findByText(/Availability rate limited visible/i)
    const rateLimitedRow = rateLimitedCell.closest('tr')
    await expect(rateLimitedRow).toHaveTextContent(/限流|Rate limited/i)
    await expect(rateLimitedRow).not.toHaveTextContent(/工作 \d+|Working \d+/i)
    await expect(rateLimitedRow).not.toHaveTextContent(/空闲|Idle/i)

    const unavailableCell = await canvas.findByText(/Availability unavailable hidden/i)
    const unavailableRow = unavailableCell.closest('tr')
    await expect(unavailableRow).not.toHaveTextContent(/工作|Working/i)
    await expect(unavailableRow).not.toHaveTextContent(/空闲|Idle/i)
  },
}
