import type { Meta, StoryObj } from '@storybook/react-vite'
import type { EffectiveRoutingRule, UpstreamAccountSummary } from '../lib/api'
import { UpstreamAccountsTable } from './UpstreamAccountsTable'

const now = '2026-03-11T12:30:00.000Z'
const defaultEffectiveRoutingRule: EffectiveRoutingRule = {
  guardEnabled: false,
  lookbackHours: null,
  maxConversations: null,
  allowCutOut: true,
  allowCutIn: true,
  sourceTagIds: [],
  sourceTagNames: [],
  guardRules: [],
}

const items: UpstreamAccountSummary[] = [
  {
    id: 11,
    kind: 'oauth_codex',
    provider: 'codex',
    displayName: 'Codex Pro - Tokyo',
    groupName: 'production',
    status: 'active',
    enabled: true,
    email: 'tokyo@example.com',
    chatgptAccountId: 'org_tokyo',
    planType: 'pro',
    lastSyncedAt: now,
    lastSuccessfulSyncAt: now,
    primaryWindow: {
      usedPercent: 42,
      usedText: '42% used',
      limitText: '5h rolling window',
      resetsAt: '2026-03-11T14:00:00.000Z',
      windowDurationMins: 300,
    },
    secondaryWindow: {
      usedPercent: 18,
      usedText: '18% used',
      limitText: '7d rolling window',
      resetsAt: '2026-03-14T00:00:00.000Z',
      windowDurationMins: 10080,
    },
    credits: {
      hasCredits: true,
      unlimited: false,
      balance: '12.80',
    },
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    localLimits: {
      primaryLimit: null,
      secondaryLimit: null,
      limitUnit: 'requests',
    },
  },
  {
    id: 12,
    kind: 'api_key_codex',
    provider: 'codex',
    displayName: 'Team key - staging',
    groupName: 'staging',
    status: 'needs_reauth',
    enabled: true,
    maskedApiKey: 'sk-live••••••c9f2',
    lastSyncedAt: '2026-03-11T08:10:00.000Z',
    lastSuccessfulSyncAt: '2026-03-11T07:48:00.000Z',
    lastError: 'refresh token expired',
    primaryWindow: {
      usedPercent: 0,
      usedText: '0 requests',
      limitText: '120 requests',
      resetsAt: '2026-03-11T13:00:00.000Z',
      windowDurationMins: 300,
    },
    secondaryWindow: {
      usedPercent: 0,
      usedText: '0 requests',
      limitText: '500 requests',
      resetsAt: '2026-03-18T00:00:00.000Z',
      windowDurationMins: 10080,
    },
    credits: {
      hasCredits: false,
      unlimited: false,
      balance: null,
    },
    tags: [],
    effectiveRoutingRule: defaultEffectiveRoutingRule,
    localLimits: {
      primaryLimit: 120,
      secondaryLimit: 500,
      limitUnit: 'requests',
    },
  },
]

const labels = {
  sync: 'Last sync',
  never: 'Never',
  group: 'Group',
  primary: '5h',
  secondary: '7d',
  nextReset: 'Reset',
  oauth: 'OAuth',
  apiKey: 'API key',
  status: (value: string) =>
    ({
      active: 'Active',
      syncing: 'Syncing',
      needs_reauth: 'Needs reauth',
      error: 'Error',
      disabled: 'Disabled',
    })[value] ?? value,
}

const meta = {
  title: 'Account Pool/Components/Upstream Accounts Table',
  component: UpstreamAccountsTable,
  tags: ['autodocs'],
  parameters: {
    layout: 'fullscreen',
  },
  decorators: [
    (Story) => (
      <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
        <div className="mx-auto max-w-6xl">
          <Story />
        </div>
      </div>
    ),
  ],
  args: {
    items,
    selectedId: 11,
    onSelect: () => undefined,
    emptyTitle: 'No upstream account yet',
    emptyDescription: 'Create an OAuth or API key account to start building the pool.',
    labels,
  },
} satisfies Meta<typeof UpstreamAccountsTable>

export default meta

type Story = StoryObj<typeof meta>

export const Default: Story = {}

export const NeedsAttentionSelected: Story = {
  args: {
    selectedId: 12,
  },
}

export const Empty: Story = {
  args: {
    items: [],
    selectedId: null,
  },
}
